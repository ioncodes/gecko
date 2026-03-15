use crate::{
    gekko::Gekko,
    mmio::{
        Mmio,
        constants::{
            DSP_BASE, DSP_END, EXI_BASE, EXI_END, GX_FIFO_BASE, GX_FIFO_END, PI_BASE, PI_END, VI_BASE, VI_END,
        },
    },
};

enum BusTarget {
    Vi,
    Pi,
    Dsp,
    Exi,
    Gx,
    Fallback,
}

#[rustfmt::skip]
fn route(phys: u32) -> (BusTarget, u32) {
    match phys {
        VI_BASE..=VI_END            => (BusTarget::Vi,  phys - VI_BASE),
        PI_BASE..=PI_END            => (BusTarget::Pi,  phys - PI_BASE),
        DSP_BASE..=DSP_END          => (BusTarget::Dsp, phys - DSP_BASE),
        EXI_BASE..=EXI_END          => (BusTarget::Exi, phys - EXI_BASE),
        GX_FIFO_BASE..=GX_FIFO_END  => (BusTarget::Gx,  phys - GX_FIFO_BASE),
        _                           => (BusTarget::Fallback, phys),
    }
}

impl Gekko {
    #[rustfmt::skip]
    pub fn read_u8(&mut self, addr: u32) -> u8 {
        let (target, offset) = route(Mmio::virt_to_phys(addr));
        match target {
            BusTarget::Vi       => self.vi.mmio_read_u8(offset),
            BusTarget::Pi       => self.pi.mmio_read_u8(offset),
            BusTarget::Dsp      => self.dsp.mmio_read_u8(offset),
            BusTarget::Exi      => self.exi.mmio_read_u8(offset),
            BusTarget::Gx       => {
                tracing::error!(addr = format!("{:08X}", addr), "Invalid GX FIFO read");
                0
            }
            BusTarget::Fallback => self.mmio.phys_read_u8(offset),
        }
    }

    #[rustfmt::skip]
    pub fn read_u16(&mut self, addr: u32) -> u16 {
        let (target, offset) = route(Mmio::virt_to_phys(addr));
        match target {
            BusTarget::Vi       => self.vi.mmio_read_u16(offset),
            BusTarget::Pi       => self.pi.mmio_read_u16(offset),
            BusTarget::Dsp      => self.dsp.mmio_read_u16(offset),
            BusTarget::Exi      => self.exi.mmio_read_u16(offset),
            BusTarget::Gx       => {
                tracing::error!(addr = format!("{:08X}", addr), "Invalid GX FIFO read");
                0
            }
            BusTarget::Fallback => self.mmio.phys_read_u16(offset),
        }
    }

    #[rustfmt::skip]
    pub fn read_u32(&mut self, addr: u32) -> u32 {
        let (target, offset) = route(Mmio::virt_to_phys(addr));
        match target {
            BusTarget::Vi       => self.vi.mmio_read_u32(offset),
            BusTarget::Pi       => self.pi.mmio_read_u32(offset),
            BusTarget::Dsp      => self.dsp.mmio_read_u32(offset),
            BusTarget::Exi      => self.exi.mmio_read_u32(offset),
            BusTarget::Gx       => {
                tracing::error!(addr = format!("{:08X}", addr), "Invalid GX FIFO read");
                0
            }
            BusTarget::Fallback => self.mmio.phys_read_u32(offset),
        }
    }

    #[rustfmt::skip]
    pub fn write_u8(&mut self, addr: u32, val: u8) {
        let (target, offset) = route(Mmio::virt_to_phys(addr));
        match target {
            BusTarget::Vi       => self.vi.mmio_write_u8(offset, val),
            BusTarget::Pi       => self.pi.mmio_write_u8(offset, val),
            BusTarget::Dsp      => {
                self.dsp.mmio_write_u8(offset, val);
                self.dsp.process_pending_dma(&mut self.mmio);
            }
            BusTarget::Exi      => {
                self.exi.mmio_write_u8(offset, val);
                self.exi.process_dma_transfers(&mut self.mmio);
            }
            BusTarget::Gx       => {
                if self.pi.is_fifo_redirected() {
                    let wptr = self.pi.fifo_wptr as usize;
                    self.mmio.ram[wptr] = val;
                    self.pi.fifo_wptr = self.pi.fifo_wptr.wrapping_add(1);
                } else {
                    self.gx.mmio_write_u8(&mut self.mmio, val);
                    self.check_gx_pe_finish();
                }
            }
            BusTarget::Fallback => self.mmio.phys_write_u8(offset, val),
        }
    }

    #[rustfmt::skip]
    pub fn write_u16(&mut self, addr: u32, val: u16) {
        let (target, offset) = route(Mmio::virt_to_phys(addr));
        match target {
            BusTarget::Vi       => self.vi.mmio_write_u16(offset, val),
            BusTarget::Pi       => self.pi.mmio_write_u16(offset, val),
            BusTarget::Dsp      => {
                self.dsp.mmio_write_u16(offset, val);
                self.dsp.process_pending_dma(&mut self.mmio);
            }
            BusTarget::Exi      => {
                self.exi.mmio_write_u16(offset, val);
                self.exi.process_dma_transfers(&mut self.mmio);
            }
            BusTarget::Gx       => {
                if self.pi.is_fifo_redirected() {
                    let wptr = self.pi.fifo_wptr as usize;
                    let bytes = val.to_be_bytes();
                    self.mmio.ram[wptr..wptr + 2].copy_from_slice(&bytes);
                    self.pi.fifo_wptr = self.pi.fifo_wptr.wrapping_add(2);
                } else {
                    self.gx.mmio_write_u16(&mut self.mmio, val);
                    self.check_gx_pe_finish();
                }
            }
            BusTarget::Fallback => self.mmio.phys_write_u16(offset, val),
        }
    }

    #[rustfmt::skip]
    pub fn write_u32(&mut self, addr: u32, val: u32) {
        let (target, offset) = route(Mmio::virt_to_phys(addr));
        match target {
            BusTarget::Vi       => self.vi.mmio_write_u32(offset, val),
            BusTarget::Pi       => self.pi.mmio_write_u32(offset, val),
            BusTarget::Dsp      => {
                self.dsp.mmio_write_u32(offset, val);
                self.dsp.process_pending_dma(&mut self.mmio);
            }
            BusTarget::Exi      => {
                self.exi.mmio_write_u32(offset, val);
                self.exi.process_dma_transfers(&mut self.mmio);
            }
            BusTarget::Gx       => {
                if self.pi.is_fifo_redirected() {
                    let wptr = self.pi.fifo_wptr as usize;
                    let bytes = val.to_be_bytes();
                    self.mmio.ram[wptr..wptr + 4].copy_from_slice(&bytes);
                    self.pi.fifo_wptr = self.pi.fifo_wptr.wrapping_add(4);
                } else {
                    self.gx.mmio_write_u32(&mut self.mmio, val);
                    self.check_gx_pe_finish();
                }
            }
            BusTarget::Fallback => self.mmio.phys_write_u32(offset, val),
        }
    }
}
