use gecko::flipper::gx::constants::*;
use gecko::flipper::gx::fifo::vertex_stride_from_cp;
use gecko::mmio::RamView;
use std::collections::HashSet;

pub const MAX_DL_DEPTH: u8 = 4;

#[derive(Clone, Debug)]
pub enum CmdKind {
    Nop,
    InvVtxCache,
    Cp {
        reg: u8,
        value: u32,
    },
    Xf {
        addr: u16,
        values: Vec<u32>,
    },
    Bp {
        reg: u8,
        value: u32,
    },
    LoadIndx {
        cmd: u8,
        index: u16,
        xf_addr: u16,
        xf_count: u8,
    },
    CallDl {
        phys_addr: u32,
        nbytes: u32,
        children: Vec<Command>,
        missing: bool,
    },
    Draw {
        cmd: u8,
        vat: u8,
        count: u16,
        stride: usize,
    },
    Unknown {
        opcode: u8,
    },
    Truncated {
        opcode: u8,
    },
}

#[derive(Clone, Debug)]
pub struct Command {
    pub offset: usize,
    pub len: usize,
    pub kind: CmdKind,
}

impl Command {
    pub fn end(&self) -> usize {
        self.offset + self.len
    }
}

#[derive(Clone, Debug, Default)]
pub struct CommandIndex {
    pub commands: Vec<Command>,
    pub truncated: bool,
}

pub struct RamOverlay<'a> {
    pub ram: RamView<'a>,
    pub updates: &'a [dff::MemoryUpdate],
}

impl RamOverlay<'_> {
    pub fn read(&self, addr: usize, len: usize, upto: u32) -> Option<Vec<u8>> {
        let mut buf = self.ram.slice(addr, len)?.to_vec();
        let end = addr + len;

        for update in self.updates {
            if update.fifo_position > upto {
                break;
            }

            let ua = update.address as usize;
            let ue = ua + update.data.len();
            if ua < end && addr < ue {
                let s = ua.max(addr);
                let e = ue.min(end);
                buf[s - addr..e - addr].copy_from_slice(&update.data[s - ua..e - ua]);
            }
        }

        Some(buf)
    }
}

pub struct SliceCtx<'a> {
    pub overlay: RamOverlay<'a>,
    pub disabled: &'a HashSet<usize>,
}

pub fn slice_frame(fifo: &[u8], cp_seed: &[u32], ctx: &SliceCtx) -> CommandIndex {
    let mut shadow = cp_seed.to_vec();
    let mut commands = Vec::new();

    let truncated = self::slice_buf(fifo, &mut shadow, ctx, 0, None, true, &mut commands);

    CommandIndex { commands, truncated }
}

fn slice_buf(
    buf: &[u8],
    shadow: &mut [u32],
    ctx: &SliceCtx,
    depth: u8,
    inherited_upto: Option<u32>,
    apply: bool,
    out: &mut Vec<Command>,
) -> bool {
    let mut pos = 0usize;
    loop {
        let remaining = buf.len() - pos;
        if remaining == 0 {
            return false;
        }

        let cmd = buf[pos];
        let offset = pos;
        let upto = inherited_upto.unwrap_or(offset as u32);
        let enabled = apply && (depth > 0 || !ctx.disabled.contains(&offset));

        let truncate = |out: &mut Vec<Command>| {
            out.push(Command {
                offset,
                len: remaining,
                kind: CmdKind::Truncated { opcode: cmd },
            });
        };

        match cmd {
            NOP_CMD => {
                out.push(Command {
                    offset,
                    len: 1,
                    kind: CmdKind::Nop,
                });

                pos += 1;
            }
            INV_VTX_CACHE_CMD => {
                out.push(Command {
                    offset,
                    len: 1,
                    kind: CmdKind::InvVtxCache,
                });

                pos += 1;
            }
            CP_CMD => {
                if remaining < 6 {
                    truncate(out);
                    return true;
                }

                let reg = buf[pos + 1];
                let value = u32::from_be_bytes(buf[pos + 2..pos + 6].try_into().unwrap());
                if enabled && (reg as usize) < shadow.len() {
                    shadow[reg as usize] = value;
                }

                out.push(Command {
                    offset,
                    len: 6,
                    kind: CmdKind::Cp { reg, value },
                });

                pos += 6;
            }
            XF_CMD => {
                if remaining < 5 {
                    truncate(out);
                    return true;
                }

                let length = u16::from_be_bytes([buf[pos + 1], buf[pos + 2]]) as usize;
                let n = length + 1;
                let total = 5 + n * 4;
                if remaining < total {
                    truncate(out);
                    return true;
                }

                let addr = u16::from_be_bytes([buf[pos + 3], buf[pos + 4]]);
                let values = (0..n)
                    .map(|i| {
                        let o = pos + 5 + i * 4;
                        u32::from_be_bytes(buf[o..o + 4].try_into().unwrap())
                    })
                    .collect();

                out.push(Command {
                    offset,
                    len: total,
                    kind: CmdKind::Xf { addr, values },
                });

                pos += total;
            }
            BP_CMD => {
                if remaining < 5 {
                    truncate(out);
                    return true;
                }

                let reg = buf[pos + 1];
                let value = u32::from_be_bytes([0, buf[pos + 2], buf[pos + 3], buf[pos + 4]]);

                out.push(Command {
                    offset,
                    len: 5,
                    kind: CmdKind::Bp { reg, value },
                });

                pos += 5;
            }
            LOAD_INDX_A_CMD | LOAD_INDX_B_CMD | LOAD_INDX_C_CMD | LOAD_INDX_D_CMD => {
                if remaining < 5 {
                    truncate(out);
                    return true;
                }

                let index = u16::from_be_bytes([buf[pos + 1], buf[pos + 2]]);
                let descriptor = u16::from_be_bytes([buf[pos + 3], buf[pos + 4]]);
                let xf_addr = descriptor & 0x0FFF;
                let xf_count = ((descriptor >> 12) & 0xF) as u8 + 1;

                out.push(Command {
                    offset,
                    len: 5,
                    kind: CmdKind::LoadIndx {
                        cmd,
                        index,
                        xf_addr,
                        xf_count,
                    },
                });

                pos += 5;
            }
            CALL_DL_CMD => {
                if remaining < 9 {
                    truncate(out);
                    return true;
                }

                let phys_addr = u32::from_be_bytes(buf[pos + 1..pos + 5].try_into().unwrap());
                let nbytes = u32::from_be_bytes(buf[pos + 5..pos + 9].try_into().unwrap());

                let addr = (phys_addr & 0x3FFFFFFF) as usize;
                let mut children = Vec::new();
                let mut missing = false;

                if depth < MAX_DL_DEPTH {
                    match ctx.overlay.read(addr, nbytes as usize, upto) {
                        Some(dl_buf) => {
                            let _ =
                                self::slice_buf(&dl_buf, shadow, ctx, depth + 1, Some(upto), enabled, &mut children);
                        }
                        None => missing = true,
                    }
                } else {
                    missing = true;
                }

                out.push(Command {
                    offset,
                    len: 9,
                    kind: CmdKind::CallDl {
                        phys_addr,
                        nbytes,
                        children,
                        missing,
                    },
                });

                pos += 9;
            }
            DRAW_COMMANDS_START..=DRAW_COMMANDS_END => {
                if remaining < 3 {
                    truncate(out);
                    return true;
                }

                let count = u16::from_be_bytes([buf[pos + 1], buf[pos + 2]]);
                let vat = cmd & 0b111;
                let stride = vertex_stride_from_cp(shadow, vat as usize);

                let total = 3 + count as usize * stride;
                if remaining < total {
                    truncate(out);
                    return true;
                }

                out.push(Command {
                    offset,
                    len: total,
                    kind: CmdKind::Draw {
                        cmd,
                        vat,
                        count,
                        stride,
                    },
                });

                pos += total;
            }
            _ => {
                out.push(Command {
                    offset,
                    len: 1,
                    kind: CmdKind::Unknown { opcode: cmd },
                });

                pos += 1;
            }
        }
    }
}
