use crate::flipper::gx::constants::DRAW_TRIANGLES_CMD;

use super::constants::{BP_CMD, CP_CMD, XF_CMD};

pub struct Fifo {
    buf: Vec<u8>,
}

impl Fifo {
    pub fn new() -> Self {
        Fifo {
            buf: Vec::with_capacity(256),
        }
    }

    pub fn push_u8(&mut self, val: u8) {
        self.buf.push(val);
    }

    pub fn push_u16(&mut self, val: u16) {
        self.buf.extend_from_slice(&val.to_be_bytes());
    }

    pub fn push_u32(&mut self, val: u32) {
        self.buf.extend_from_slice(&val.to_be_bytes());
    }

    /// Drain complete commands from the FIFO, returning each as a `FifoCmd`.
    pub fn drain(&mut self) -> Vec<FifoCmd> {
        let mut cmds = Vec::new();
        let mut pos = 0;

        loop {
            let remaining = self.buf.len() - pos;
            if remaining == 0 {
                break;
            }

            let cmd = self.buf[pos];
            match cmd {
                CP_CMD => {
                    // 1 cmd + 1 addr + 4 data = 6 bytes
                    if remaining < 6 {
                        break;
                    }
                    let data: [u8; 5] = self.buf[pos + 1..pos + 6].try_into().unwrap();
                    cmds.push(FifoCmd::Cp(data));
                    pos += 6;
                }
                XF_CMD => {
                    // 1 cmd + 2 length + 2 addr = 5 byte header minimum
                    if remaining < 5 {
                        break;
                    }
                    let length = u16::from_be_bytes([self.buf[pos + 1], self.buf[pos + 2]]) as usize;
                    let n = length + 1;
                    let total = 5 + n * 4;
                    if remaining < total {
                        break;
                    }
                    let data = self.buf[pos + 1..pos + total].to_vec();
                    cmds.push(FifoCmd::Xf(data));
                    pos += total;
                }
                BP_CMD => {
                    // 1 cmd + 4 data = 5 bytes
                    if remaining < 5 {
                        break;
                    }
                    let data: [u8; 4] = self.buf[pos + 1..pos + 5].try_into().unwrap();
                    cmds.push(FifoCmd::Bp(data));
                    pos += 5;
                }
                DRAW_TRIANGLES_CMD => {
                    // 1 command + minimum 2 vertex count
                    // [cmd_byte] [count_hi] [count_lo] [vertex_0_data...] [vertex_1_data...] ...
                    if remaining < 3 {
                        break;
                    }

                    let count = u16::from_be_bytes([self.buf[pos + 1], self.buf[pos + 2]]) as usize;
                    let total = 3 + count * 2; // TODO: Assuming each vertex is 2 bytes
                    if remaining < total {
                        break;
                    }

                    let vertex_data = self.buf[pos + 3..pos + total].to_vec();
                    cmds.push(FifoCmd::DrawTriangles(vertex_data));

                    pos += total;
                }
                _ => {
                    tracing::error!(cmd = format!("{cmd:02X}"), "unknown FIFO command");
                    pos += 1;
                }
            }
        }

        if pos > 0 {
            self.buf.drain(..pos);
        }

        cmds
    }
}

#[derive(Debug)]
pub enum FifoCmd {
    Cp([u8; 5]),
    Xf(Vec<u8>),
    Bp([u8; 4]),
    DrawTriangles(Vec<u8>),
}
