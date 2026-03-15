use super::constants::{BP_CMD, CALL_DL_CMD, CP_CMD, INV_VTX_CACHE_CMD, NOP_CMD, XF_CMD};
use crate::flipper::gx::{
    Gx,
    constants::{DRAW_COMMANDS_END, DRAW_COMMANDS_START, VATA_REG, VCD_HI_REG, VCD_LO_REG},
    regs::{AttributeType, VatA, VcdHi, VcdLo},
};
use std::io::{Cursor, Read};

impl Gx {
    pub fn push_u8(&mut self, val: u8) {
        self.fifo.push(val);
    }

    pub fn push_u16(&mut self, val: u16) {
        self.fifo.extend_from_slice(&val.to_be_bytes());
    }

    pub fn push_u32(&mut self, val: u32) {
        self.fifo.extend_from_slice(&val.to_be_bytes());
    }

    /// Drain complete commands from the FIFO, returning each as a `FifoCmd`.
    pub fn drain(&mut self) -> Vec<FifoCmd> {
        let mut cmds = Vec::new();
        let mut cur = Cursor::new(&self.fifo);

        loop {
            let pos = cur.position() as usize;
            let remaining = self.fifo.len() - pos;
            if remaining == 0 {
                break;
            }

            let mut cmd_buf = [0u8; 1];
            cur.read_exact(&mut cmd_buf).unwrap();
            let cmd = cmd_buf[0];

            match cmd {
                NOP_CMD | INV_VTX_CACHE_CMD => {
                    // NOP / vertex cache invalidate, skip
                }
                CALL_DL_CMD => {
                    // 4-byte physical address + 4-byte size = 8 bytes payload
                    if remaining < 9 {
                        cur.set_position(pos as u64);
                        break;
                    }
                    let mut addr_buf = [0u8; 4];
                    let mut size_buf = [0u8; 4];
                    cur.read_exact(&mut addr_buf).unwrap();
                    cur.read_exact(&mut size_buf).unwrap();
                    let phys_addr = u32::from_be_bytes(addr_buf);
                    let nbytes = u32::from_be_bytes(size_buf);
                    cmds.push(FifoCmd::CallDisplayList { phys_addr, nbytes });
                }
                CP_CMD => {
                    // 1 addr + 4 data = 5 bytes
                    if remaining < 6 {
                        cur.set_position(pos as u64);
                        break;
                    }
                    let mut data = [0u8; 5];
                    cur.read_exact(&mut data).unwrap();
                    cmds.push(FifoCmd::Cp(data));
                }
                XF_CMD => {
                    // 2 length + 2 addr = 4 byte header
                    if remaining < 5 {
                        cur.set_position(pos as u64);
                        break;
                    }
                    let mut header = [0u8; 4];
                    cur.read_exact(&mut header).unwrap();
                    let length = u16::from_be_bytes([header[0], header[1]]) as usize;
                    let n = length + 1;
                    let total = 5 + n * 4;
                    if remaining < total {
                        cur.set_position(pos as u64);
                        break;
                    }
                    let mut rest = vec![0u8; n * 4];
                    cur.read_exact(&mut rest).unwrap();
                    let data = [header.as_slice(), rest.as_slice()].concat();
                    cmds.push(FifoCmd::Xf(data));
                }
                BP_CMD => {
                    // 4 data bytes
                    if remaining < 5 {
                        cur.set_position(pos as u64);
                        break;
                    }
                    let mut data = [0u8; 4];
                    cur.read_exact(&mut data).unwrap();
                    cmds.push(FifoCmd::Bp(data));
                }
                DRAW_COMMANDS_START..=DRAW_COMMANDS_END => {
                    // [count_hi] [count_lo] [vertex_0_data...] ...
                    if remaining < 3 {
                        cur.set_position(pos as u64);
                        break;
                    }
                    let mut count_buf = [0u8; 2];
                    cur.read_exact(&mut count_buf).unwrap();
                    let count = u16::from_be_bytes(count_buf) as usize;
                    let vertex_format_index = (cmd & 0b111) as usize;
                    let vertex_data_len = count * self.vertex_stride(vertex_format_index);
                    let total = 3 + vertex_data_len;
                    if remaining < total {
                        cur.set_position(pos as u64);
                        break;
                    }
                    let mut vertex_data = vec![0u8; vertex_data_len];
                    cur.read_exact(&mut vertex_data).unwrap();
                    cmds.push(FifoCmd::DrawCall(cmd, vertex_data));
                }
                _ => {
                    tracing::error!(cmd = format!("{cmd:02X}"), "unknown FIFO command");
                }
            }
        }

        let consumed = cur.position() as usize;
        if consumed > 0 {
            self.fifo.drain(..consumed);
        }

        cmds
    }

    fn vertex_stride(&self, vertex_format_index: usize) -> usize {
        let vcd_lo = VcdLo::from_raw(self.cp_regs[VCD_LO_REG + vertex_format_index]);
        let vcd_hi = VcdHi::from_raw(self.cp_regs[VCD_HI_REG + vertex_format_index]);
        let vat_a = VatA::from_raw(self.cp_regs[VATA_REG + vertex_format_index]);

        let tex0_size = match vcd_hi.tex0() {
            AttributeType::Direct => vat_a.tex0_data_size(),
            AttributeType::Index8 => 1,
            AttributeType::Index16 => 2,
            AttributeType::None => 0,
        };

        let pos_size = match vcd_lo.position() {
            AttributeType::Direct => vat_a.pos_data_size(),
            AttributeType::Index8 => 1,
            AttributeType::Index16 => 2,
            AttributeType::None => 0,
        };

        let nrm_size = match vcd_lo.normal() {
            AttributeType::Direct => vat_a.nrm_data_size(),
            AttributeType::Index8 => 1,
            AttributeType::Index16 => 2,
            AttributeType::None => 0,
        };

        let clr0_size = match vcd_lo.color0() {
            AttributeType::Direct => vat_a.clr0_data_size(),
            AttributeType::Index8 => 1,
            AttributeType::Index16 => 2,
            AttributeType::None => 0,
        };

        pos_size + nrm_size + clr0_size + tex0_size
    }
}

#[derive(Debug)]
pub enum FifoCmd {
    Cp([u8; 5]),
    Xf(Vec<u8>),
    Bp([u8; 4]),
    CallDisplayList { phys_addr: u32, nbytes: u32 },
    DrawCall(u8, Vec<u8>),
}
