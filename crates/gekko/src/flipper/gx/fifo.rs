use crate::flipper::gx::{Gx, constants::{DRAW_TRIANGLES_CMD, VCD_LO_REG}, regs::VcdLo};

use super::constants::{BP_CMD, CP_CMD, XF_CMD};

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
        let mut pos = 0;

        loop {
            let remaining = self.fifo.len() - pos;
            if remaining == 0 {
                break;
            }

            let cmd = self.fifo[pos];
            match cmd {
                CP_CMD => {
                    // 1 cmd + 1 addr + 4 data = 6 bytes
                    if remaining < 6 {
                        break;
                    }
                    let data: [u8; 5] = self.fifo[pos + 1..pos + 6].try_into().unwrap();
                    cmds.push(FifoCmd::Cp(data));
                    pos += 6;
                }
                XF_CMD => {
                    // 1 cmd + 2 length + 2 addr = 5 byte header minimum
                    if remaining < 5 {
                        break;
                    }
                    let length = u16::from_be_bytes([self.fifo[pos + 1], self.fifo[pos + 2]]) as usize;
                    let n = length + 1;
                    let total = 5 + n * 4;
                    if remaining < total {
                        break;
                    }
                    let data = self.fifo[pos + 1..pos + total].to_vec();
                    cmds.push(FifoCmd::Xf(data));
                    pos += total;
                }
                BP_CMD => {
                    // 1 cmd + 4 data = 5 bytes
                    if remaining < 5 {
                        break;
                    }
                    let data: [u8; 4] = self.fifo[pos + 1..pos + 5].try_into().unwrap();
                    cmds.push(FifoCmd::Bp(data));
                    pos += 5;
                }
                DRAW_TRIANGLES_CMD => {
                    // 1 command + minimum 2 vertex count
                    // [cmd_byte] [count_hi] [count_lo] [vertex_0_data...] [vertex_1_data...] ...
                    if remaining < 3 {
                        break;
                    }

                    let count = u16::from_be_bytes([self.fifo[pos + 1], self.fifo[pos + 2]]) as usize;
                    let vertex_format_index = (cmd & 0b111) as usize;
                    let total = 3 + count * self.vertex_stride(vertex_format_index);
                    if remaining < total {
                        break;
                    }

                    let vertex_data = self.fifo[pos + 3..pos + total].to_vec();
                    cmds.push(FifoCmd::DrawTriangles(cmd, vertex_data));

                    pos += total;
                }
                _ => {
                    tracing::error!(cmd = format!("{cmd:02X}"), "unknown FIFO command");
                    pos += 1;
                }
            }
        }

        if pos > 0 {
            self.fifo.drain(..pos);
        }

        cmds
    }

    fn vertex_stride(&self, vertex_format_index: usize) -> usize {
        let vcd_lo = VcdLo::from_raw(self.cp_regs[VCD_LO_REG + vertex_format_index]);
        vcd_lo.position().size() + vcd_lo.color0().size()
    }
}

#[derive(Debug)]
pub enum FifoCmd {
    Cp([u8; 5]),
    Xf(Vec<u8>),
    Bp([u8; 4]),
    DrawTriangles(u8, Vec<u8>),
}
