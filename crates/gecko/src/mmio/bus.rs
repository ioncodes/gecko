use crate::gamecube::GameCube;
#[cfg(feature = "hooks")]
use crate::hooks::HookFlags;
use crate::mmio::MmioRw;
use crate::mmio::constants::{
    AI_BASE, AI_END, CP_BASE, CP_END, DI_BASE, DI_END, DSP_BASE, DSP_END, EXI_BASE, EXI_END, GX_FIFO_BASE, GX_FIFO_END,
    IPL_BASE, IPL_END, MI_BASE, MI_END, PE_BASE, PE_END, PI_BASE, PI_END, RAM_END, SI_BASE, SI_END, VI_BASE, VI_END,
};

enum BusTarget {
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
fn route_mmio(phys: u32) -> (BusTarget, u32) {
    match phys {
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
        #[cfg(feature = "hooks")]
        if $self.hook_flags.contains(HookFlags::BUS_READ_PRE) {
            if $self.hook_filters.bus_read_pre.matches($addr, $phys) {
                if let Some(mut host) = $self.hook_host.take() {
                    if let Some(val) = host.on_bus_read_pre($self, $addr, $phys, $size) {
                        $self.sync_pending_hook_state(host.as_mut());
                        $self.hook_host = Some(host);
                        return val as _;
                    }
                    $self.sync_pending_hook_state(host.as_mut());
                    $self.hook_host = Some(host);
                }
            }
        }

        #[allow(unused_mut)]
        let mut result = $body;

        #[cfg(feature = "hooks")]
        if $self.hook_flags.contains(HookFlags::BUS_READ_POST) {
            if $self.hook_filters.bus_read_post.matches($addr, $phys) {
                if let Some(mut host) = $self.hook_host.take() {
                    result = host.on_bus_read_post($self, $addr, $phys, $size, result as u32) as _;
                    $self.sync_pending_hook_state(host.as_mut());
                    $self.hook_host = Some(host);
                }
            }
        }

        result
    }};
}

macro_rules! bus_write_hooks {
    ($self:ident, $addr:ident, $phys:ident, $size:literal, $val:ident, $body:expr) => {{
        #[cfg(feature = "hooks")]
        let $val = if $self.hook_flags.contains(HookFlags::BUS_WRITE_PRE) {
            if $self.hook_filters.bus_write_pre.matches($addr, $phys) {
                if let Some(mut host) = $self.hook_host.take() {
                    let v = host.on_bus_write_pre($self, $addr, $phys, $size, $val as u32) as _;
                    $self.sync_pending_hook_state(host.as_mut());
                    $self.hook_host = Some(host);
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

        #[cfg(feature = "hooks")]
        if $self.hook_flags.contains(HookFlags::BUS_WRITE_POST) {
            if $self.hook_filters.bus_write_post.matches($addr, $phys) {
                if let Some(mut host) = $self.hook_host.take() {
                    host.on_bus_write_post($self, $addr, $phys, $size, $val as u32);
                    $self.sync_pending_hook_state(host.as_mut());
                    $self.hook_host = Some(host);
                }
            }
        }
    }};
}

impl GameCube {
    /// Translate a virtual address to physical using DBAT registers.
    /// Falls back to simple masking if no BAT matches.
    #[inline(always)]
    fn translate_addr(&self, ea: u32) -> u32 {
        // Fast path: 0x80/0xC0 with simple mask covers most accesses
        let top = ea >> 28;
        if top == 0x8 || top == 0xC {
            return ea & 0x3FFFFFFF;
        }

        // Check all 4 DBATs
        let dbats = [
            (self.cpu.spr.dbat0u, self.cpu.spr.dbat0l),
            (self.cpu.spr.dbat1u, self.cpu.spr.dbat1l),
            (self.cpu.spr.dbat2u, self.cpu.spr.dbat2l),
            (self.cpu.spr.dbat3u, self.cpu.spr.dbat3l),
        ];

        for (batu, batl) in dbats {
            // Valid in supervisor (Vs) or problem (Vp) mode
            if (batu & 0x3) == 0 {
                continue;
            }

            let bl = (batu >> 2) & 0x7FF; // block length mask
            let bepi = batu >> 17; // upper 15 bits of EA
            let ea_upper = ea >> 17;

            // Check if EA falls within this BAT's range
            // Match condition: (EA >> 17) & ~BL == BEPI & ~BL
            if (ea_upper & !bl) == (bepi & !bl) {
                let brpn = batl >> 17; // upper 15 bits of PA
                let pa = (brpn | (ea_upper & bl)) << 17 | (ea & 0x1FFFF);
                return pa;
            }
        }

        // No BAT match, fall back to simple mask
        ea & 0x3FFFFFFF
    }

    // Read fast path for RAM access

    #[inline(always)]
    pub fn read_u8(&mut self, addr: u32) -> u8 {
        let phys = self.translate_addr(addr);
        bus_read_hooks!(self, addr, phys, 1, {
            if phys <= RAM_END {
                self.mmio.ram_read_u8(phys)
            } else {
                self.read_u8_mmio(phys, addr)
            }
        })
    }

    #[inline(always)]
    pub fn read_u16(&mut self, addr: u32) -> u16 {
        let phys = self.translate_addr(addr);
        bus_read_hooks!(self, addr, phys, 2, {
            if phys <= RAM_END - 1 {
                self.mmio.ram_read_u16(phys)
            } else {
                self.read_u16_mmio(phys, addr)
            }
        })
    }

    #[inline(always)]
    pub fn read_u32(&mut self, addr: u32) -> u32 {
        let phys = self.translate_addr(addr);
        bus_read_hooks!(self, addr, phys, 4, {
            if phys <= RAM_END - 3 {
                self.mmio.ram_read_u32(phys)
            } else {
                self.read_u32_mmio(phys, addr)
            }
        })
    }

    // Write fast path for RAM access

    #[inline(always)]
    pub fn write_u8(&mut self, addr: u32, val: u8) {
        let phys = self.translate_addr(addr);
        bus_write_hooks!(self, addr, phys, 1, val, {
            if phys <= RAM_END {
                self.mmio.ram_write_u8(phys, val);
            } else {
                self.write_u8_mmio(phys, val);
            }
        });
    }

    #[inline(always)]
    pub fn write_u16(&mut self, addr: u32, val: u16) {
        let phys = self.translate_addr(addr);
        bus_write_hooks!(self, addr, phys, 2, val, {
            if phys <= RAM_END - 1 {
                self.mmio.ram_write_u16(phys, val);
            } else {
                self.write_u16_mmio(phys, val);
            }
        });
    }

    #[inline(always)]
    pub fn write_u32(&mut self, addr: u32, val: u32) {
        let phys = self.translate_addr(addr);
        bus_write_hooks!(self, addr, phys, 4, val, {
            if phys <= RAM_END - 3 {
                self.mmio.ram_write_u32(phys, val);
            } else {
                self.write_u32_mmio(phys, val);
            }
        });
    }

    // Slow path for MMIO

    #[rustfmt::skip]
    #[inline(never)]
    fn read_u8_mmio(&mut self, phys: u32, addr: u32) -> u8 {
        let (target, offset) = route_mmio(phys);
        match target {
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
            BusTarget::Dsp      => crate::flipper::dsp::dsp_read(self, phys, 1).unwrap_or(0) as u8,
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
            BusTarget::Ipl      => self.mmio.phys_read_u8(phys),
            BusTarget::Fallback => self.mmio.phys_read_u8(phys),
        }
    }

    #[rustfmt::skip]
    #[inline(never)]
    fn read_u16_mmio(&mut self, phys: u32, addr: u32) -> u16 {
        let (target, offset) = route_mmio(phys);
        match target {
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
            BusTarget::Dsp      => crate::flipper::dsp::dsp_read(self, phys, 2).unwrap_or(0) as u16,
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
            BusTarget::Ipl      => self.mmio.phys_read_u16(phys),
            BusTarget::Fallback => self.mmio.phys_read_u16(phys),
        }
    }

    #[rustfmt::skip]
    #[inline(never)]
    fn read_u32_mmio(&mut self, phys: u32, addr: u32) -> u32 {
        let (target, offset) = route_mmio(phys);
        match target {
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
            BusTarget::Dsp      => crate::flipper::dsp::dsp_read(self, phys, 4).unwrap_or(0),
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
            BusTarget::Ipl      => self.mmio.phys_read_u32(phys),
            BusTarget::Fallback => self.mmio.phys_read_u32(phys),
        }
    }

    #[rustfmt::skip]
    #[inline(never)]
    fn write_u8_mmio(&mut self, phys: u32, val: u8) {
        let (target, offset) = route_mmio(phys);
        match target {
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
            BusTarget::Dsp      => { crate::flipper::dsp::dsp_write(self, phys, 1, val as u32); }
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
                let wptr = self.pi.fifo_wptr as usize;
                self.mmio.ram[wptr] = val;
                self.pi.advance_fifo_wptr(1);
                if self.cp.control.gp_link_enable() {
                    self.gx.mmio_write_u8(&mut self.mmio, val);
                    self.check_gx_pe_interrupts();
                }
            }
            BusTarget::Ipl      => self.mmio.phys_write_u8(phys, val),
            BusTarget::Fallback => self.mmio.phys_write_u8(phys, val),
        }
    }

    #[rustfmt::skip]
    #[inline(never)]
    fn write_u16_mmio(&mut self, phys: u32, val: u16) {
        let (target, offset) = route_mmio(phys);
        match target {
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
            BusTarget::Dsp      => { crate::flipper::dsp::dsp_write(self, phys, 2, val as u32); }
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
                let wptr = self.pi.fifo_wptr as usize;
                let bytes = val.to_be_bytes();
                self.mmio.ram[wptr..wptr + 2].copy_from_slice(&bytes);
                self.pi.advance_fifo_wptr(2);
                if self.cp.control.gp_link_enable() {
                    self.gx.mmio_write_u16(&mut self.mmio, val);
                    self.check_gx_pe_interrupts();
                }
            }
            BusTarget::Ipl      => self.mmio.phys_write_u16(phys, val),
            BusTarget::Fallback => self.mmio.phys_write_u16(phys, val),
        }
    }

    #[rustfmt::skip]
    #[inline(never)]
    fn write_u32_mmio(&mut self, phys: u32, val: u32) {
        let (target, offset) = route_mmio(phys);
        match target {
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
            BusTarget::Dsp      => { crate::flipper::dsp::dsp_write(self, phys, 4, val); }
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
                let wptr = self.pi.fifo_wptr as usize;
                let bytes = val.to_be_bytes();
                self.mmio.ram[wptr..wptr + 4].copy_from_slice(&bytes);
                self.pi.advance_fifo_wptr(4);
                if self.cp.control.gp_link_enable() {
                    self.gx.mmio_write_u32(&mut self.mmio, val);
                    self.check_gx_pe_interrupts();
                }
            }
            BusTarget::Ipl      => self.mmio.phys_write_u32(phys, val),
            BusTarget::Fallback => self.mmio.phys_write_u32(phys, val),
        }
    }
}
