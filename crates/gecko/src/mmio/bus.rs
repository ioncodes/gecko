#[cfg(feature = "scripting")]
use crate::scripting::HookFlags;
use crate::{
    gamecube::GameCube,
    mmio::{
        Mmio, MmioRw,
        constants::{
            AI_BASE, AI_END, CP_BASE, CP_END, DI_BASE, DI_END, DSP_BASE, DSP_END, EXI_BASE, EXI_END, GX_FIFO_BASE,
            GX_FIFO_END, IPL_BASE, IPL_END, MI_BASE, MI_END, PE_BASE, PE_END, PI_BASE, PI_END, RAM_BASE, RAM_END,
            SI_BASE, SI_END, VI_BASE, VI_END,
        },
    },
};

enum BusTarget {
    Ram,
    Cp,
    Vi,
    Pe,
    Pi,
    Mi,
    Dsp,
    Di,
    Si,
    Exi,
    Ai,
    Gx,
    Ipl,
    Fallback,
}

#[rustfmt::skip]
#[inline(always)]
fn route(phys: u32) -> (BusTarget, u32) {
    match phys {
        RAM_BASE..=RAM_END          => (BusTarget::Ram, phys),
        CP_BASE..=CP_END            => (BusTarget::Cp,  phys - CP_BASE),
        PE_BASE..=PE_END            => (BusTarget::Pe,  phys - PE_BASE),
        VI_BASE..=VI_END            => (BusTarget::Vi,  phys - VI_BASE),
        PI_BASE..=PI_END            => (BusTarget::Pi,  phys - PI_BASE),
        MI_BASE..=MI_END            => (BusTarget::Mi,  phys - MI_BASE),
        DSP_BASE..=DSP_END          => (BusTarget::Dsp, phys - DSP_BASE),
        DI_BASE..=DI_END            => (BusTarget::Di,  phys - DI_BASE),
        SI_BASE..=SI_END            => (BusTarget::Si,  phys - SI_BASE),
        EXI_BASE..=EXI_END          => (BusTarget::Exi, phys - EXI_BASE),
        AI_BASE..=AI_END            => (BusTarget::Ai,  phys - AI_BASE),
        IPL_BASE..=IPL_END          => (BusTarget::Ipl, phys),
        GX_FIFO_BASE..=GX_FIFO_END  => (BusTarget::Gx,  phys - GX_FIFO_BASE),
        _                           => (BusTarget::Fallback, phys),
    }
}

macro_rules! bus_read_hooks {
    ($self:ident, $addr:ident, $phys:ident, $size:literal, $body:expr) => {{
        #[cfg(feature = "scripting")]
        if $self.script_hook_flags.contains(HookFlags::BUS_READ_PRE) {
            if $self.script_hook_filters.bus_read_pre.matches($addr, $phys) {
                if let Some(mut host) = $self.script_host.take() {
                    if let Some(val) = host.on_bus_read_pre($self, $addr, $phys, $size) {
                        $self.sync_pending_script_hook_state(host.as_mut());
                        $self.script_host = Some(host);
                        return val as _;
                    }
                    $self.sync_pending_script_hook_state(host.as_mut());
                    $self.script_host = Some(host);
                }
            }
        }

        #[allow(unused_mut)]
        let mut result = $body;

        #[cfg(feature = "scripting")]
        if $self.script_hook_flags.contains(HookFlags::BUS_READ_POST) {
            if $self.script_hook_filters.bus_read_post.matches($addr, $phys) {
                if let Some(mut host) = $self.script_host.take() {
                    result = host.on_bus_read_post($self, $addr, $phys, $size, result as u32) as _;
                    $self.sync_pending_script_hook_state(host.as_mut());
                    $self.script_host = Some(host);
                }
            }
        }

        result
    }};
}

macro_rules! bus_write_hooks {
    ($self:ident, $addr:ident, $phys:ident, $size:literal, $val:ident, $body:expr) => {{
        #[cfg(feature = "scripting")]
        let $val = if $self.script_hook_flags.contains(HookFlags::BUS_WRITE_PRE) {
            if $self.script_hook_filters.bus_write_pre.matches($addr, $phys) {
                if let Some(mut host) = $self.script_host.take() {
                    let v = host.on_bus_write_pre($self, $addr, $phys, $size, $val as u32) as _;
                    $self.sync_pending_script_hook_state(host.as_mut());
                    $self.script_host = Some(host);
                    v
                } else {
                    $val
                }
            } else {
                $val
            }
        } else {
            $val
        };

        $body;

        #[cfg(feature = "scripting")]
        if $self.script_hook_flags.contains(HookFlags::BUS_WRITE_POST) {
            if $self.script_hook_filters.bus_write_post.matches($addr, $phys) {
                if let Some(mut host) = $self.script_host.take() {
                    host.on_bus_write_post($self, $addr, $phys, $size, $val as u32);
                    $self.sync_pending_script_hook_state(host.as_mut());
                    $self.script_host = Some(host);
                }
            }
        }
    }};
}

impl GameCube {
    #[rustfmt::skip]
    #[inline(always)]
    pub fn read_u8(&mut self, addr: u32) -> u8 {
        let phys = Mmio::virt_to_phys(addr);
        bus_read_hooks!(self, addr, phys, 1, {
            let (target, offset) = route(phys);
            match target {
                BusTarget::Ram      => self.mmio.ram_read_u8(offset),
                BusTarget::Cp       => self.cp.mmio_read_u8(offset),
                BusTarget::Vi       => {
                    if offset == 0x2E {
                        return (self.vi.dph_value(self.scheduler.cycles) >> 8) as u8;
                    }
                    if offset == 0x2F {
                        return self.vi.dph_value(self.scheduler.cycles) as u8;
                    }
                    self.vi.mmio_read_u8(offset)
                }
                BusTarget::Pe       => self.pe.mmio_read_u8(offset),
                BusTarget::Pi       => self.pi.mmio_read_u8(offset),
                BusTarget::Mi       => self.mi.mmio_read_u8(offset),
                BusTarget::Dsp      => self.dsp.mmio_read_u8(offset),
                BusTarget::Di       => self.di.mmio_read_u8(offset),
                BusTarget::Si       => self.si.mmio_read_u8(offset),
                BusTarget::Exi      => self.exi.mmio_read_u8(offset),
                BusTarget::Ai       => {
                    if offset == 0x08 {
                        return self.ai.sample_count(self.scheduler.cycles) as u8;
                    }
                    self.ai.mmio_read_u8(offset)
                }
                BusTarget::Gx       => {
                    tracing::error!(addr = format!("{:08X}", addr), "Invalid GX FIFO read");
                    0
                }
                BusTarget::Ipl      => self.mmio.phys_read_u8(offset),
                BusTarget::Fallback => self.mmio.phys_read_u8(offset),
            }
        })
    }

    #[rustfmt::skip]
    #[inline(always)]
    pub fn read_u16(&mut self, addr: u32) -> u16 {
        let phys = Mmio::virt_to_phys(addr);
        bus_read_hooks!(self, addr, phys, 2, {
            let (target, offset) = route(phys);
            match target {
                BusTarget::Ram      => self.mmio.ram_read_u16(offset),
                BusTarget::Cp       => self.cp.mmio_read_u16(offset),
                BusTarget::Vi       => {
                    if offset == 0x2E {
                        return self.vi.dph_value(self.scheduler.cycles);
                    }
                    self.vi.mmio_read_u16(offset)
                }
                BusTarget::Pe       => self.pe.mmio_read_u16(offset),
                BusTarget::Pi       => self.pi.mmio_read_u16(offset),
                BusTarget::Mi       => self.mi.mmio_read_u16(offset),
                BusTarget::Dsp      => self.dsp.mmio_read_u16(offset),
                BusTarget::Di       => self.di.mmio_read_u16(offset),
                BusTarget::Si       => self.si.mmio_read_u16(offset),
                BusTarget::Exi      => self.exi.mmio_read_u16(offset),
                BusTarget::Ai       => {
                    if offset == 0x08 || offset == 0x0A {
                        return self.ai.sample_count(self.scheduler.cycles) as u16;
                    }
                    self.ai.mmio_read_u16(offset)
                }
                BusTarget::Gx       => {
                    tracing::error!(addr = format!("{:08X}", addr), "Invalid GX FIFO read");
                    0
                }
                BusTarget::Ipl      => self.mmio.phys_read_u16(offset),
                BusTarget::Fallback => self.mmio.phys_read_u16(offset),
            }
        })
    }

    #[rustfmt::skip]
    #[inline(always)]
    pub fn read_u32(&mut self, addr: u32) -> u32 {
        let phys = Mmio::virt_to_phys(addr);
        bus_read_hooks!(self, addr, phys, 4, {
            let (target, offset) = route(phys);
            match target {
                BusTarget::Ram      => self.mmio.ram_read_u32(offset),
                BusTarget::Cp       => self.cp.mmio_read_u32(offset),
                BusTarget::Vi       => {
                    if offset == 0x2C {
                        let dpv = self.vi.mmio_read_u16(0x2C) as u32;
                        let dph = self.vi.dph_value(self.scheduler.cycles) as u32;
                        return (dpv << 16) | dph;
                    }
                    self.vi.mmio_read_u32(offset)
                }
                BusTarget::Pe       => self.pe.mmio_read_u32(offset),
                BusTarget::Pi       => self.pi.mmio_read_u32(offset),
                BusTarget::Mi       => self.mi.mmio_read_u32(offset),
                BusTarget::Dsp      => self.dsp.mmio_read_u32(offset),
                BusTarget::Di       => self.di.mmio_read_u32(offset),
                BusTarget::Si       => self.si.mmio_read_u32(offset),
                BusTarget::Exi      => self.exi.mmio_read_u32(offset),
                BusTarget::Ai       => {
                    if offset == 0x08 {
                        return self.ai.sample_count(self.scheduler.cycles);
                    }
                    self.ai.mmio_read_u32(offset)
                }
                BusTarget::Gx       => {
                    tracing::error!(addr = format!("{:08X}", addr), "Invalid GX FIFO read");
                    0
                }
                BusTarget::Ipl      => self.mmio.phys_read_u32(offset),
                BusTarget::Fallback => self.mmio.phys_read_u32(offset),
            }
        })
    }

    #[rustfmt::skip]
    #[inline(always)]
    pub fn write_u8(&mut self, addr: u32, val: u8) {
        let phys = Mmio::virt_to_phys(addr);
        bus_write_hooks!(self, addr, phys, 1, val, {
            let (target, offset) = route(phys);
            match target {
                BusTarget::Ram      => self.mmio.ram_write_u8(offset, val),
                BusTarget::Cp       => {
                    self.cp.mmio_write_u8(offset, val);
                    self.check_cp_interrupts();
                }
                BusTarget::Vi       => {
                    self.vi.mmio_write_u8(offset, val);
                    self.maybe_schedule_vi_half_line();
                    if (0x30..=0x3F).contains(&offset) {
                        self.check_vi_interrupts();
                    }
                }
                BusTarget::Pe       => {
                    self.pe.mmio_write_u8(offset, val);
                    self.check_pe_interrupts();
                }
                BusTarget::Pi       => self.pi.mmio_write_u8(offset, val),
                BusTarget::Mi       => self.mi.mmio_write_u8(offset, val),
                BusTarget::Dsp      => {
                    self.dsp.mmio_write_u8(offset, val);
                    self.dsp.process_pending_dma(&mut self.mmio);
                    self.check_dsp_interrupts();
                }
                BusTarget::Di       => {
                    self.di.mmio_write_u8(offset, val);
                    self.start_dvd_transfer();
                    self.check_di_interrupts();
                }
                BusTarget::Si       => {
                    self.si.mmio_write_u8(offset, val);
                    self.check_si_interrupts();
                }
                BusTarget::Exi      => {
                    self.exi.mmio_write_u8(offset, val);
                    self.exi.process_cs_changes();
                    self.exi.process_dma_transfers(&mut self.mmio);
                    self.check_exi_interrupts();
                }
                BusTarget::Ai       => {
                    self.ai.mmio_write_u8(offset, val);
                    self.check_sample_counter_reset();
                    self.check_ai_interrupts();
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
                BusTarget::Ipl      => self.mmio.phys_write_u8(offset, val),
                BusTarget::Fallback => self.mmio.phys_write_u8(offset, val),
            }
        });
    }

    #[rustfmt::skip]
    #[inline(always)]
    pub fn write_u16(&mut self, addr: u32, val: u16) {
        let phys = Mmio::virt_to_phys(addr);
        bus_write_hooks!(self, addr, phys, 2, val, {
            let (target, offset) = route(phys);
            match target {
                BusTarget::Ram      => self.mmio.ram_write_u16(offset, val),
                BusTarget::Cp       => {
                    self.cp.mmio_write_u16(offset, val);
                    self.check_cp_interrupts();
                }
                BusTarget::Vi       => {
                    self.vi.mmio_write_u16(offset, val);
                    self.maybe_schedule_vi_half_line();
                    if (0x30..=0x3F).contains(&offset) {
                        self.check_vi_interrupts();
                    }
                }
                BusTarget::Pe       => {
                    self.pe.mmio_write_u16(offset, val);
                    self.check_pe_interrupts();
                }
                BusTarget::Pi       => self.pi.mmio_write_u16(offset, val),
                BusTarget::Mi       => self.mi.mmio_write_u16(offset, val),
                BusTarget::Dsp      => {
                    self.dsp.mmio_write_u16(offset, val);
                    self.dsp.process_pending_dma(&mut self.mmio);
                    self.check_dsp_interrupts();
                }
                BusTarget::Di       => {
                    self.di.mmio_write_u16(offset, val);
                    self.start_dvd_transfer();
                    self.check_di_interrupts();
                }
                BusTarget::Si       => {
                    self.si.mmio_write_u16(offset, val);
                    self.check_si_interrupts();
                }
                BusTarget::Exi      => {
                    self.exi.mmio_write_u16(offset, val);
                    self.exi.process_cs_changes();
                    self.exi.process_dma_transfers(&mut self.mmio);
                    self.check_exi_interrupts();
                }
                BusTarget::Ai       => {
                    self.ai.mmio_write_u16(offset, val);
                    self.check_sample_counter_reset();
                    self.check_ai_interrupts();
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
                BusTarget::Ipl      => self.mmio.phys_write_u16(offset, val),
                BusTarget::Fallback => self.mmio.phys_write_u16(offset, val),
            }
        });
    }

    #[rustfmt::skip]
    #[inline(always)]
    pub fn write_u32(&mut self, addr: u32, val: u32) {
        let phys = Mmio::virt_to_phys(addr);
        bus_write_hooks!(self, addr, phys, 4, val, {
            let (target, offset) = route(phys);
            match target {
                BusTarget::Ram      => self.mmio.ram_write_u32(offset, val),
                BusTarget::Cp       => {
                    self.cp.mmio_write_u32(offset, val);
                    self.check_cp_interrupts();
                }
                BusTarget::Vi       => {
                    self.vi.mmio_write_u32(offset, val);
                    self.maybe_schedule_vi_half_line();
                    if (0x30..=0x3F).contains(&offset) {
                        self.check_vi_interrupts();
                    }
                }
                BusTarget::Pe       => {
                    self.pe.mmio_write_u32(offset, val);
                    self.check_pe_interrupts();
                }
                BusTarget::Pi       => self.pi.mmio_write_u32(offset, val),
                BusTarget::Mi       => self.mi.mmio_write_u32(offset, val),
                BusTarget::Dsp      => {
                    self.dsp.mmio_write_u32(offset, val);
                    self.dsp.process_pending_dma(&mut self.mmio);
                    self.check_dsp_interrupts();
                }
                BusTarget::Di       => {
                    self.di.mmio_write_u32(offset, val);
                    self.start_dvd_transfer();
                    self.check_di_interrupts();
                }
                BusTarget::Si       => {
                    self.si.mmio_write_u32(offset, val);
                    self.check_si_interrupts();
                }
                BusTarget::Exi      => {
                    self.exi.mmio_write_u32(offset, val);
                    self.exi.process_cs_changes();
                    self.exi.process_dma_transfers(&mut self.mmio);
                    self.check_exi_interrupts();
                }
                BusTarget::Ai       => {
                    self.ai.mmio_write_u32(offset, val);
                    self.check_sample_counter_reset();
                    self.check_ai_interrupts();
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
                BusTarget::Ipl      => self.mmio.phys_write_u32(offset, val),
                BusTarget::Fallback => self.mmio.phys_write_u32(offset, val),
            }
        });
    }
}
