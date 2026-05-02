use super::constants::*;
use super::regs::*;
use crate::flipper::gx::GraphicsProcessor;
use crate::host::RenderSink;
use crate::mmio::Mmio;
use crate::system::SystemId;

impl GraphicsProcessor {
    pub fn push_u8(&mut self, val: u8) {
        self.fifo.push(val);
    }

    pub fn push_u16(&mut self, val: u16) {
        self.fifo.extend_from_slice(&val.to_be_bytes());
    }

    pub fn push_u32(&mut self, val: u32) {
        self.fifo.extend_from_slice(&val.to_be_bytes());
    }

    /// Streaming GP FIFO command processor: parses and dispatches each
    /// command inline so state writes (CP/XF/BP) take effect before the
    /// next command's parse decisions read them.
    ///
    /// Partial commands are left in `self.fifo` for the next push to
    /// complete: when a command's payload isn't fully present, we `break`
    /// without advancing `pos`, then drain only the consumed prefix below.
    pub fn drain_fifo<const SYSTEM: SystemId>(&mut self, mmio: &mut Mmio<SYSTEM>, renderer: &mut dyn RenderSink) {
        let mut pos = 0usize;
        loop {
            let remaining = self.fifo.len() - pos;
            if remaining == 0 {
                break;
            }
            let cmd = self.fifo[pos];

            match cmd {
                NOP_CMD | INV_VTX_CACHE_CMD => {
                    pos += 1;
                }
                CP_CMD => {
                    if remaining < 6 {
                        break;
                    }
                    let mut data = [0u8; 5];
                    data.copy_from_slice(&self.fifo[pos + 1..pos + 6]);
                    pos += 6;
                    self.load_cp(&data);
                }
                XF_CMD => {
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
                    pos += total;
                    self.load_xf(renderer, &data);
                }
                BP_CMD => {
                    if remaining < 5 {
                        break;
                    }
                    let mut data = [0u8; 4];
                    data.copy_from_slice(&self.fifo[pos + 1..pos + 5]);
                    pos += 5;
                    let mut view = mmio.ram_view_mut();
                    self.load_bp(renderer, &mut view, &data);
                }
                LOAD_INDX_A_CMD | LOAD_INDX_B_CMD | LOAD_INDX_C_CMD | LOAD_INDX_D_CMD => {
                    if remaining < 5 {
                        break;
                    }
                    let payload: [u8; 4] = self.fifo[pos + 1..pos + 5].try_into().unwrap();
                    pos += 5;

                    let index = u16::from_be_bytes([payload[0], payload[1]]);
                    let descriptor = u16::from_be_bytes([payload[2], payload[3]]);
                    let xf_addr = descriptor & 0x0FFF;
                    let xf_count = ((descriptor >> 12) & 0xF) as u8 + 1;

                    let cp_array_index = match cmd {
                        LOAD_INDX_A_CMD => ARRAY_POS_NRM_MTX,
                        LOAD_INDX_B_CMD => ARRAY_NRM_MTX,
                        LOAD_INDX_C_CMD => ARRAY_POST_MTX,
                        LOAD_INDX_D_CMD => ARRAY_LIGHT,
                        _ => unreachable!(),
                    } as u8;

                    let view = mmio.ram_view();
                    self.load_indexed_xf(renderer, &view, cp_array_index, index, xf_addr, xf_count);
                }
                CALL_DL_CMD => {
                    if remaining < 9 {
                        break;
                    }
                    let phys_addr = u32::from_be_bytes(self.fifo[pos + 1..pos + 5].try_into().unwrap());
                    let nbytes = u32::from_be_bytes(self.fifo[pos + 5..pos + 9].try_into().unwrap());
                    pos += 9;

                    let addr = (phys_addr & 0x3FFFFFFF) as usize;
                    let len = nbytes as usize;
                    let dl = match mmio.ram_view().slice(addr, len) {
                        Some(slice) => slice.to_vec(),
                        None => {
                            tracing::warn!(
                                addr = format!("{addr:#010X}"),
                                len,
                                "CallDisplayList: source not mapped to MEM1/MEM2, skipping"
                            );
                            continue;
                        }
                    };
                    self.execute_display_list(mmio, renderer, &dl);
                }
                DRAW_COMMANDS_START..=DRAW_COMMANDS_END => {
                    if remaining < 3 {
                        break;
                    }
                    let count = u16::from_be_bytes([self.fifo[pos + 1], self.fifo[pos + 2]]) as usize;
                    let vertex_format_index = (cmd & 0b111) as usize;
                    let vertex_data_len = count * self.vertex_stride(vertex_format_index);
                    let total = 3 + vertex_data_len;
                    if remaining < total {
                        break;
                    }
                    let vertex_data = self.fifo[pos + 3..pos + total].to_vec();
                    pos += total;
                    self.create_draw_call(mmio, renderer, cmd, vertex_data);
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
    }

    fn vertex_stride(&self, vertex_format_index: usize) -> usize {
        // VCD is global state (single register), VAT is per-format
        let vcd_lo = VcdLo::from_raw(self.cp_regs[VCD_LO_REG]);
        let vcd_hi = VcdHi::from_raw(self.cp_regs[VCD_HI_REG]);
        let vat_a = VatA::from_raw(self.cp_regs[VATA_REG + vertex_format_index]);
        let vat_b = VatB::from_raw(self.cp_regs[VATB_REG + vertex_format_index]);
        let vat_c = VatC::from_raw(self.cp_regs[VATC_REG + vertex_format_index]);

        let attr_size = |attr: AttributeType, direct_size: usize| -> usize {
            match attr {
                AttributeType::Direct => direct_size,
                AttributeType::Index8 => 1,
                AttributeType::Index16 => 2,
                AttributeType::None => 0,
            }
        };

        vcd_lo.mtx_idx_count()
            + attr_size(vcd_lo.position(), vat_a.pos_data_size())
            + vat_a.nrm_stream_size(vcd_lo.normal())
            + attr_size(vcd_lo.color0(), vat_a.clr0_data_size())
            + attr_size(vcd_lo.color1(), vat_a.clr1_data_size())
            + attr_size(vcd_hi.tex0(), vat_a.tex0_data_size())
            + attr_size(vcd_hi.tex1(), vat_b.tex1_data_size())
            + attr_size(vcd_hi.tex2(), vat_b.tex2_data_size())
            + attr_size(vcd_hi.tex3(), vat_b.tex3_data_size())
            + attr_size(vcd_hi.tex4(), vat_b.tex4_data_size())
            + attr_size(vcd_hi.tex5(), vat_c.tex5_data_size())
            + attr_size(vcd_hi.tex6(), vat_c.tex6_data_size())
            + attr_size(vcd_hi.tex7(), vat_c.tex7_data_size())
    }
}
