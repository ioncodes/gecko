use super::constants::*;
use super::regs::*;
use crate::flipper::gx::GraphicsProcessor;
use crate::host::RenderSink;
use crate::mmio::Mmio;
use crate::system::SystemId;

impl GraphicsProcessor {
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn drain_fifo<const SYSTEM: SystemId>(&mut self, mmio: &mut Mmio<SYSTEM>, renderer: &mut dyn RenderSink) {
        let mut fifo = std::mem::take(&mut self.fifo);

        let pos = self::drain_buf::<SYSTEM>(self, mmio, renderer, &fifo);
        if pos > 0 {
            fifo.drain(..pos);
        }

        self.fifo = fifo;
    }
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
fn drain_buf<const SYSTEM: SystemId>(
    gp: &mut GraphicsProcessor,
    mmio: &mut Mmio<SYSTEM>,
    renderer: &mut dyn RenderSink,
    fifo: &[u8],
) -> usize {
    let mut pos = 0usize;
    loop {
        let remaining = fifo.len() - pos;
        if remaining == 0 {
            break;
        }

        let cmd = fifo[pos];

        match cmd {
            NOP_CMD | INV_VTX_CACHE_CMD => {
                pos += 1;
            }
            CP_CMD => {
                if remaining < 6 {
                    break;
                }

                let mut data = [0u8; 5];
                data.copy_from_slice(&fifo[pos + 1..pos + 6]);

                pos += 6;

                gp.load_cp(&data);
            }
            XF_CMD => {
                if remaining < 5 {
                    break;
                }

                let length = u16::from_be_bytes([fifo[pos + 1], fifo[pos + 2]]) as usize;
                let n = length + 1;
                let total = 5 + n * 4;
                if remaining < total {
                    break;
                }

                let data = &fifo[pos + 1..pos + total];

                gp.load_xf(renderer, data);

                pos += total;
            }
            BP_CMD => {
                if remaining < 5 {
                    break;
                }

                let mut data = [0u8; 4];
                data.copy_from_slice(&fifo[pos + 1..pos + 5]);

                pos += 5;

                let mut view = mmio.ram_view_mut();
                gp.load_bp(renderer, &mut view, &data);
            }
            LOAD_INDX_A_CMD | LOAD_INDX_B_CMD | LOAD_INDX_C_CMD | LOAD_INDX_D_CMD => {
                if remaining < 5 {
                    break;
                }

                let payload: [u8; 4] = fifo[pos + 1..pos + 5].try_into().unwrap();
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
                gp.load_indexed_xf(renderer, &view, cp_array_index, index, xf_addr, xf_count);
            }
            CALL_DL_CMD => {
                if remaining < 9 {
                    break;
                }

                let phys_addr = u32::from_be_bytes(fifo[pos + 1..pos + 5].try_into().unwrap());
                let nbytes = u32::from_be_bytes(fifo[pos + 5..pos + 9].try_into().unwrap());
                pos += 9;

                let addr = (phys_addr & 0x3FFFFFFF) as usize;
                let len = nbytes as usize;
                let dl_buf: Vec<u8> = match mmio.ram_view().slice(addr, len) {
                    Some(slice) => {
                        let mut buf = std::mem::take(&mut gp.dl_scratch);
                        buf.clear();
                        buf.extend_from_slice(slice);
                        buf
                    }
                    None => {
                        tracing::warn!(
                            addr = format_args!("{addr:#010X}").to_string(),
                            len,
                            "CallDisplayList: source not mapped"
                        );
                        continue;
                    }
                };

                let _ = self::drain_buf::<SYSTEM>(gp, mmio, renderer, &dl_buf);
                gp.dl_scratch = dl_buf;
            }
            DRAW_COMMANDS_START..=DRAW_COMMANDS_END => {
                if remaining < 3 {
                    break;
                }

                let count = u16::from_be_bytes([fifo[pos + 1], fifo[pos + 2]]) as usize;
                let vertex_format_index = (cmd & 0b111) as usize;
                let vertex_data_len = count * gp.vertex_stride(vertex_format_index);
                let total = 3 + vertex_data_len;
                if remaining < total {
                    break;
                }

                let vertex_data = &fifo[pos + 3..pos + total];
                gp.create_draw_call(mmio, renderer, cmd, vertex_data);
                pos += total;
            }
            _ => {
                tracing::error!(cmd = cmd, "unknown FIFO command");
                pos += 1;
            }
        }
    }

    pos
}

impl GraphicsProcessor {
    fn vertex_stride(&self, vertex_format_index: usize) -> usize {
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
