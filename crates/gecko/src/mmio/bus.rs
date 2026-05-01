#[cfg(feature = "hooks")]
use crate::hooks::HookFlags;
use crate::mmio::constants::{
    AI_BASE, AI_END, CP_BASE, CP_END, DI_BASE, DI_END, DSP_BASE, DSP_END, EXI_BASE, EXI_END, GX_FIFO_BASE, GX_FIFO_END,
    HOLLYWOOD_COMPAT_BASE, HOLLYWOOD_COMPAT_END, HOLLYWOOD_GPIO_ARM_BASE, HOLLYWOOD_GPIO_ARM_END,
    HOLLYWOOD_GPIO_PPC_BASE, HOLLYWOOD_GPIO_PPC_END, HOLLYWOOD_IRQ_BASE, HOLLYWOOD_IRQ_END, HOLLYWOOD_PLL_AI_BASE,
    HOLLYWOOD_PLL_AI_END, HW_HOLLYWOOD_BASE, HW_HOLLYWOOD_END, HW_REG_BASE, HW_REG_END, IPC_BASE, IPC_END, MI_BASE,
    MI_END, PE_BASE, PE_END, PI_BASE, PI_END, RAM_END, SI_BASE, SI_END, VI_BASE, VI_END,
};
use crate::system::{System, SystemId, WII};

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

impl<const SYSTEM: SystemId> System<SYSTEM> {
    /// Translate a virtual address to physical using DBAT registers.
    /// Falls back to simple masking if no BAT matches.
    #[inline(always)]
    fn translate_addr(&self, ea: u32) -> u32 {
        Self::reroute_hollywood_mirrors(self.bat_translate(ea))
    }

    #[inline(always)]
    fn bat_translate(&self, ea: u32) -> u32 {
        // Fast path: 0x80/0xC0 with simple mask covers most accesses
        let top = ea >> 28;
        if top == 0x8 || top == 0xC {
            return ea & 0x3FFFFFFF;
        }

        // Check DBATs. GC has 4, Wii's Broadway has 8 (extended BATs).
        let mut dbats: [(u32, u32); 8] = [
            (self.gekko.spr.dbat0u, self.gekko.spr.dbat0l),
            (self.gekko.spr.dbat1u, self.gekko.spr.dbat1l),
            (self.gekko.spr.dbat2u, self.gekko.spr.dbat2l),
            (self.gekko.spr.dbat3u, self.gekko.spr.dbat3l),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
        ];
        if SYSTEM == WII {
            dbats[4] = (self.gekko.spr.dbat4u, self.gekko.spr.dbat4l);
            dbats[5] = (self.gekko.spr.dbat5u, self.gekko.spr.dbat5l);
            dbats[6] = (self.gekko.spr.dbat6u, self.gekko.spr.dbat6l);
            dbats[7] = (self.gekko.spr.dbat7u, self.gekko.spr.dbat7l);
        }

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

    /// Holly wood mirrors DI/SI/EXI/AI at 0x0D006000..0x0D006FFF
    /// (as per Flipper at 0x0C006xxx).
    /// Simply route these back.
    #[inline(always)]
    fn reroute_hollywood_mirrors(phys: u32) -> u32 {
        if SYSTEM == WII && matches!(phys, 0x0D00_6000..=0x0D00_6FFF) {
            phys - 0x0100_0000
        } else {
            phys
        }
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
                self.write_u8_mmio(phys, addr, val);
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
                self.write_u16_mmio(phys, addr, val);
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
                self.write_u32_mmio(phys, addr, val);
            }
        });
    }

    // Slow path for MMIO

    #[inline(never)]
    fn read_u8_mmio(&mut self, phys: u32, addr: u32) -> u8 {
        match phys {
            CP_BASE..=CP_END => crate::flipper::cp::cp_read(self, phys, 1).unwrap_or_else(|| {
                tracing::warn!(
                    device = "CP",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 1,
                    "Unimplemented MMIO read"
                );
                0
            }) as u8,
            PE_BASE..=PE_END => crate::flipper::pe::pe_read(self, phys, 1).unwrap_or_else(|| {
                tracing::warn!(
                    device = "PE",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 1,
                    "Unimplemented MMIO read"
                );
                0
            }) as u8,
            VI_BASE..=VI_END => crate::flipper::vi::vi_read(self, phys, 1).unwrap_or_else(|| {
                tracing::warn!(
                    device = "VI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 1,
                    "Unimplemented MMIO read"
                );
                0
            }) as u8,
            PI_BASE..=PI_END => crate::flipper::pi::pi_read(self, phys, 1).unwrap_or_else(|| {
                tracing::warn!(
                    device = "PI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 1,
                    "Unimplemented MMIO read"
                );
                0
            }) as u8,
            MI_BASE..=MI_END => crate::flipper::mi::mi_read(self, phys, 1).unwrap_or_else(|| {
                tracing::warn!(
                    device = "MI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 1,
                    "Unimplemented MMIO read"
                );
                0
            }) as u8,
            DSP_BASE..=DSP_END => crate::flipper::dsp::dsp_read(self, phys, 1).unwrap_or_else(|| {
                tracing::warn!(
                    device = "DSP",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 1,
                    "Unimplemented MMIO read"
                );
                0
            }) as u8,
            DI_BASE..=DI_END => crate::dvd::di_read(self, phys, 1).unwrap_or_else(|| {
                tracing::warn!(
                    device = "DI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 1,
                    "Unimplemented MMIO read"
                );
                0
            }) as u8,
            SI_BASE..=SI_END => crate::flipper::si::si_read(self, phys, 1).unwrap_or_else(|| {
                tracing::warn!(
                    device = "SI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 1,
                    "Unimplemented MMIO read"
                );
                0
            }) as u8,
            EXI_BASE..=EXI_END => crate::flipper::exi::exi_read(self, phys, 1).unwrap_or_else(|| {
                tracing::warn!(
                    device = "EXI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 1,
                    "Unimplemented MMIO read"
                );
                0
            }) as u8,
            AI_BASE..=AI_END => crate::flipper::ai::ai_read(self, phys, 1).unwrap_or_else(|| {
                tracing::warn!(
                    device = "AI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 1,
                    "Unimplemented MMIO read"
                );
                0
            }) as u8,
            IPC_BASE..=IPC_END if SYSTEM == WII => crate::hollywood::ipc::ipc_read(self, phys, 1).unwrap_or(0) as u8,
            HOLLYWOOD_IRQ_BASE..=HOLLYWOOD_IRQ_END if SYSTEM == WII => {
                crate::hollywood::irq::irq_read(self, phys, 1).unwrap_or(0) as u8
            }
            HOLLYWOOD_GPIO_PPC_BASE..=HOLLYWOOD_GPIO_PPC_END if SYSTEM == WII => {
                crate::hollywood::gpio::gpio_read(self, phys, 1).unwrap_or(0) as u8
            }
            HOLLYWOOD_GPIO_ARM_BASE..=HOLLYWOOD_GPIO_ARM_END if SYSTEM == WII => {
                crate::hollywood::gpio::gpio_read(self, phys, 1).unwrap_or(0) as u8
            }
            HOLLYWOOD_COMPAT_BASE..=HOLLYWOOD_COMPAT_END | HOLLYWOOD_PLL_AI_BASE..=HOLLYWOOD_PLL_AI_END
                if SYSTEM == WII =>
            {
                crate::hollywood::compat::compat_read(self, phys, 1).unwrap_or(0) as u8
            }
            HW_HOLLYWOOD_BASE..=HW_HOLLYWOOD_END if SYSTEM == WII => {
                tracing::warn!(
                    device = "HOLLYWOOD",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 1,
                    "Unimplemented MMIO read"
                );
                self.mmio.phys_read_u8(phys)
            }
            GX_FIFO_BASE..=GX_FIFO_END => {
                tracing::error!(addr = format!("{:08X}", addr), "Invalid GX FIFO read");
                0
            }
            HW_REG_BASE..=HW_REG_END => {
                tracing::warn!(
                    device = "HW_REG",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 1,
                    "Unimplemented MMIO read"
                );
                self.mmio.phys_read_u8(phys)
            }
            _ => self.mmio.phys_read_u8(phys),
        }
    }

    #[inline(never)]
    fn read_u16_mmio(&mut self, phys: u32, addr: u32) -> u16 {
        match phys {
            CP_BASE..=CP_END => crate::flipper::cp::cp_read(self, phys, 2).unwrap_or_else(|| {
                tracing::warn!(
                    device = "CP",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 2,
                    "Unimplemented MMIO read"
                );
                0
            }) as u16,
            PE_BASE..=PE_END => crate::flipper::pe::pe_read(self, phys, 2).unwrap_or_else(|| {
                tracing::warn!(
                    device = "PE",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 2,
                    "Unimplemented MMIO read"
                );
                0
            }) as u16,
            VI_BASE..=VI_END => crate::flipper::vi::vi_read(self, phys, 2).unwrap_or_else(|| {
                tracing::warn!(
                    device = "VI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 2,
                    "Unimplemented MMIO read"
                );
                0
            }) as u16,
            PI_BASE..=PI_END => crate::flipper::pi::pi_read(self, phys, 2).unwrap_or_else(|| {
                tracing::warn!(
                    device = "PI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 2,
                    "Unimplemented MMIO read"
                );
                0
            }) as u16,
            MI_BASE..=MI_END => crate::flipper::mi::mi_read(self, phys, 2).unwrap_or_else(|| {
                tracing::warn!(
                    device = "MI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 2,
                    "Unimplemented MMIO read"
                );
                0
            }) as u16,
            DSP_BASE..=DSP_END => crate::flipper::dsp::dsp_read(self, phys, 2).unwrap_or_else(|| {
                tracing::warn!(
                    device = "DSP",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 2,
                    "Unimplemented MMIO read"
                );
                0
            }) as u16,
            DI_BASE..=DI_END => crate::dvd::di_read(self, phys, 2).unwrap_or_else(|| {
                tracing::warn!(
                    device = "DI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 2,
                    "Unimplemented MMIO read"
                );
                0
            }) as u16,
            SI_BASE..=SI_END => crate::flipper::si::si_read(self, phys, 2).unwrap_or_else(|| {
                tracing::warn!(
                    device = "SI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 2,
                    "Unimplemented MMIO read"
                );
                0
            }) as u16,
            EXI_BASE..=EXI_END => crate::flipper::exi::exi_read(self, phys, 2).unwrap_or_else(|| {
                tracing::warn!(
                    device = "EXI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 2,
                    "Unimplemented MMIO read"
                );
                0
            }) as u16,
            AI_BASE..=AI_END => crate::flipper::ai::ai_read(self, phys, 2).unwrap_or_else(|| {
                tracing::warn!(
                    device = "AI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 2,
                    "Unimplemented MMIO read"
                );
                0
            }) as u16,
            IPC_BASE..=IPC_END if SYSTEM == WII => crate::hollywood::ipc::ipc_read(self, phys, 2).unwrap_or(0) as u16,
            HOLLYWOOD_IRQ_BASE..=HOLLYWOOD_IRQ_END if SYSTEM == WII => {
                crate::hollywood::irq::irq_read(self, phys, 2).unwrap_or(0) as u16
            }
            HOLLYWOOD_GPIO_PPC_BASE..=HOLLYWOOD_GPIO_PPC_END if SYSTEM == WII => {
                crate::hollywood::gpio::gpio_read(self, phys, 2).unwrap_or(0) as u16
            }
            HOLLYWOOD_GPIO_ARM_BASE..=HOLLYWOOD_GPIO_ARM_END if SYSTEM == WII => {
                crate::hollywood::gpio::gpio_read(self, phys, 2).unwrap_or(0) as u16
            }
            HOLLYWOOD_COMPAT_BASE..=HOLLYWOOD_COMPAT_END | HOLLYWOOD_PLL_AI_BASE..=HOLLYWOOD_PLL_AI_END
                if SYSTEM == WII =>
            {
                crate::hollywood::compat::compat_read(self, phys, 2).unwrap_or(0) as u16
            }
            HW_HOLLYWOOD_BASE..=HW_HOLLYWOOD_END if SYSTEM == WII => {
                tracing::warn!(
                    device = "HOLLYWOOD",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 2,
                    "Unimplemented MMIO read"
                );
                self.mmio.phys_read_u16(phys)
            }
            GX_FIFO_BASE..=GX_FIFO_END => {
                tracing::error!(addr = format!("{:08X}", addr), "Invalid GX FIFO read");
                0
            }
            HW_REG_BASE..=HW_REG_END => {
                tracing::warn!(
                    device = "HW_REG",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 2,
                    "Unimplemented MMIO read"
                );
                self.mmio.phys_read_u16(phys)
            }
            _ => self.mmio.phys_read_u16(phys),
        }
    }

    #[inline(never)]
    fn read_u32_mmio(&mut self, phys: u32, addr: u32) -> u32 {
        match phys {
            CP_BASE..=CP_END => crate::flipper::cp::cp_read(self, phys, 4).unwrap_or_else(|| {
                tracing::warn!(
                    device = "CP",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 4,
                    "Unimplemented MMIO read"
                );
                0
            }),
            PE_BASE..=PE_END => crate::flipper::pe::pe_read(self, phys, 4).unwrap_or_else(|| {
                tracing::warn!(
                    device = "PE",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 4,
                    "Unimplemented MMIO read"
                );
                0
            }),
            VI_BASE..=VI_END => crate::flipper::vi::vi_read(self, phys, 4).unwrap_or_else(|| {
                tracing::warn!(
                    device = "VI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 4,
                    "Unimplemented MMIO read"
                );
                0
            }),
            PI_BASE..=PI_END => crate::flipper::pi::pi_read(self, phys, 4).unwrap_or_else(|| {
                tracing::warn!(
                    device = "PI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 4,
                    "Unimplemented MMIO read"
                );
                0
            }),
            MI_BASE..=MI_END => crate::flipper::mi::mi_read(self, phys, 4).unwrap_or_else(|| {
                tracing::warn!(
                    device = "MI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 4,
                    "Unimplemented MMIO read"
                );
                0
            }),
            DSP_BASE..=DSP_END => crate::flipper::dsp::dsp_read(self, phys, 4).unwrap_or_else(|| {
                tracing::warn!(
                    device = "DSP",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 4,
                    "Unimplemented MMIO read"
                );
                0
            }),
            DI_BASE..=DI_END => crate::dvd::di_read(self, phys, 4).unwrap_or_else(|| {
                tracing::warn!(
                    device = "DI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 4,
                    "Unimplemented MMIO read"
                );
                0
            }),
            SI_BASE..=SI_END => crate::flipper::si::si_read(self, phys, 4).unwrap_or_else(|| {
                tracing::warn!(
                    device = "SI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 4,
                    "Unimplemented MMIO read"
                );
                0
            }),
            EXI_BASE..=EXI_END => crate::flipper::exi::exi_read(self, phys, 4).unwrap_or_else(|| {
                tracing::warn!(
                    device = "EXI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 4,
                    "Unimplemented MMIO read"
                );
                0
            }),
            AI_BASE..=AI_END => crate::flipper::ai::ai_read(self, phys, 4).unwrap_or_else(|| {
                tracing::warn!(
                    device = "AI",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 4,
                    "Unimplemented MMIO read"
                );
                0
            }),
            IPC_BASE..=IPC_END if SYSTEM == WII => crate::hollywood::ipc::ipc_read(self, phys, 4).unwrap_or(0),
            HOLLYWOOD_IRQ_BASE..=HOLLYWOOD_IRQ_END if SYSTEM == WII => {
                crate::hollywood::irq::irq_read(self, phys, 4).unwrap_or(0)
            }
            HOLLYWOOD_GPIO_PPC_BASE..=HOLLYWOOD_GPIO_PPC_END if SYSTEM == WII => {
                crate::hollywood::gpio::gpio_read(self, phys, 4).unwrap_or(0)
            }
            HOLLYWOOD_GPIO_ARM_BASE..=HOLLYWOOD_GPIO_ARM_END if SYSTEM == WII => {
                crate::hollywood::gpio::gpio_read(self, phys, 4).unwrap_or(0)
            }
            HOLLYWOOD_COMPAT_BASE..=HOLLYWOOD_COMPAT_END | HOLLYWOOD_PLL_AI_BASE..=HOLLYWOOD_PLL_AI_END
                if SYSTEM == WII =>
            {
                crate::hollywood::compat::compat_read(self, phys, 4).unwrap_or(0)
            }
            HW_HOLLYWOOD_BASE..=HW_HOLLYWOOD_END if SYSTEM == WII => {
                tracing::warn!(
                    device = "HOLLYWOOD",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 4,
                    "Unimplemented MMIO read"
                );
                self.mmio.phys_read_u32(phys)
            }
            GX_FIFO_BASE..=GX_FIFO_END => {
                tracing::error!(addr = format!("{:08X}", addr), "Invalid GX FIFO read");
                0
            }
            HW_REG_BASE..=HW_REG_END => {
                tracing::warn!(
                    device = "HW_REG",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    size = 4,
                    "Unimplemented MMIO read"
                );
                self.mmio.phys_read_u32(phys)
            }
            _ => self.mmio.phys_read_u32(phys),
        }
    }

    #[inline(never)]
    fn write_u8_mmio(&mut self, phys: u32, addr: u32, val: u8) {
        let raw = val as u32;
        match phys {
            CP_BASE..=CP_END => {
                if !crate::flipper::cp::cp_write(self, phys, 1, raw) {
                    tracing::warn!(
                        device = "CP",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 1,
                        "Unimplemented MMIO write"
                    );
                }
            }
            PE_BASE..=PE_END => {
                if !crate::flipper::pe::pe_write(self, phys, 1, raw) {
                    tracing::warn!(
                        device = "PE",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 1,
                        "Unimplemented MMIO write"
                    );
                }
            }
            VI_BASE..=VI_END => {
                if !crate::flipper::vi::vi_write(self, phys, 1, raw) {
                    tracing::warn!(
                        device = "VI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 1,
                        "Unimplemented MMIO write"
                    );
                }
            }
            PI_BASE..=PI_END => {
                if !crate::flipper::pi::pi_write(self, phys, 1, raw) {
                    tracing::warn!(
                        device = "PI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 1,
                        "Unimplemented MMIO write"
                    );
                }
            }
            MI_BASE..=MI_END => {
                if !crate::flipper::mi::mi_write(self, phys, 1, raw) {
                    tracing::warn!(
                        device = "MI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 1,
                        "Unimplemented MMIO write"
                    );
                }
            }
            DSP_BASE..=DSP_END => {
                if !crate::flipper::dsp::dsp_write(self, phys, 1, raw) {
                    tracing::warn!(
                        device = "DSP",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 1,
                        "Unimplemented MMIO write"
                    );
                }
            }
            DI_BASE..=DI_END => {
                if !crate::dvd::di_write(self, phys, 1, raw) {
                    tracing::warn!(
                        device = "DI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 1,
                        "Unimplemented MMIO write"
                    );
                }
            }
            SI_BASE..=SI_END => {
                if !crate::flipper::si::si_write(self, phys, 1, raw) {
                    tracing::warn!(
                        device = "SI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 1,
                        "Unimplemented MMIO write"
                    );
                }
            }
            EXI_BASE..=EXI_END => {
                if !crate::flipper::exi::exi_write(self, phys, 1, raw) {
                    tracing::warn!(
                        device = "EXI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 1,
                        "Unimplemented MMIO write"
                    );
                }
            }
            AI_BASE..=AI_END => {
                if !crate::flipper::ai::ai_write(self, phys, 1, raw) {
                    tracing::warn!(
                        device = "AI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 1,
                        "Unimplemented MMIO write"
                    );
                }
            }
            IPC_BASE..=IPC_END if SYSTEM == WII => {
                crate::hollywood::ipc::ipc_write(self, phys, 1, raw);
            }
            HOLLYWOOD_IRQ_BASE..=HOLLYWOOD_IRQ_END if SYSTEM == WII => {
                crate::hollywood::irq::irq_write(self, phys, 1, raw);
            }
            HOLLYWOOD_GPIO_PPC_BASE..=HOLLYWOOD_GPIO_PPC_END if SYSTEM == WII => {
                crate::hollywood::gpio::gpio_write(self, phys, 1, raw);
            }
            HOLLYWOOD_GPIO_ARM_BASE..=HOLLYWOOD_GPIO_ARM_END if SYSTEM == WII => {
                crate::hollywood::gpio::gpio_write(self, phys, 1, raw);
            }
            HOLLYWOOD_COMPAT_BASE..=HOLLYWOOD_COMPAT_END | HOLLYWOOD_PLL_AI_BASE..=HOLLYWOOD_PLL_AI_END
                if SYSTEM == WII =>
            {
                crate::hollywood::compat::compat_write(self, phys, 1, raw);
            }
            HW_HOLLYWOOD_BASE..=HW_HOLLYWOOD_END if SYSTEM == WII => {
                tracing::warn!(
                    device = "HOLLYWOOD",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    val = format!("{raw:08X}"),
                    size = 1,
                    "Unimplemented MMIO write"
                );
                self.mmio.phys_write_u8(phys, val);
            }
            GX_FIFO_BASE..=GX_FIFO_END => {
                let wptr = self.pi.fifo_wptr;
                self.mmio.phys_write_u8(wptr, val);
                self.pi.advance_fifo_wptr(1);
                if self.cp.control.gp_link_enable() {
                    self.gx.mmio_write_u8(&mut self.mmio, self.render_sink.as_mut(), val);
                    self.check_gx_pe_interrupts();
                }
            }
            HW_REG_BASE..=HW_REG_END => {
                tracing::warn!(
                    device = "HW_REG",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    val = format!("{raw:08X}"),
                    size = 1,
                    "Unimplemented MMIO write"
                );
                self.mmio.phys_write_u8(phys, val);
            }
            _ => self.mmio.phys_write_u8(phys, val),
        }
    }

    #[inline(never)]
    fn write_u16_mmio(&mut self, phys: u32, addr: u32, val: u16) {
        let raw = val as u32;
        match phys {
            CP_BASE..=CP_END => {
                if !crate::flipper::cp::cp_write(self, phys, 2, raw) {
                    tracing::warn!(
                        device = "CP",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 2,
                        "Unimplemented MMIO write"
                    );
                }
            }
            PE_BASE..=PE_END => {
                if !crate::flipper::pe::pe_write(self, phys, 2, raw) {
                    tracing::warn!(
                        device = "PE",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 2,
                        "Unimplemented MMIO write"
                    );
                }
            }
            VI_BASE..=VI_END => {
                if !crate::flipper::vi::vi_write(self, phys, 2, raw) {
                    tracing::warn!(
                        device = "VI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 2,
                        "Unimplemented MMIO write"
                    );
                }
            }
            PI_BASE..=PI_END => {
                if !crate::flipper::pi::pi_write(self, phys, 2, raw) {
                    tracing::warn!(
                        device = "PI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 2,
                        "Unimplemented MMIO write"
                    );
                }
            }
            MI_BASE..=MI_END => {
                if !crate::flipper::mi::mi_write(self, phys, 2, raw) {
                    tracing::warn!(
                        device = "MI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 2,
                        "Unimplemented MMIO write"
                    );
                }
            }
            DSP_BASE..=DSP_END => {
                if !crate::flipper::dsp::dsp_write(self, phys, 2, raw) {
                    tracing::warn!(
                        device = "DSP",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 2,
                        "Unimplemented MMIO write"
                    );
                }
            }
            DI_BASE..=DI_END => {
                if !crate::dvd::di_write(self, phys, 2, raw) {
                    tracing::warn!(
                        device = "DI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 2,
                        "Unimplemented MMIO write"
                    );
                }
            }
            SI_BASE..=SI_END => {
                if !crate::flipper::si::si_write(self, phys, 2, raw) {
                    tracing::warn!(
                        device = "SI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 2,
                        "Unimplemented MMIO write"
                    );
                }
            }
            EXI_BASE..=EXI_END => {
                if !crate::flipper::exi::exi_write(self, phys, 2, raw) {
                    tracing::warn!(
                        device = "EXI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 2,
                        "Unimplemented MMIO write"
                    );
                }
            }
            AI_BASE..=AI_END => {
                if !crate::flipper::ai::ai_write(self, phys, 2, raw) {
                    tracing::warn!(
                        device = "AI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{raw:08X}"),
                        size = 2,
                        "Unimplemented MMIO write"
                    );
                }
            }
            IPC_BASE..=IPC_END if SYSTEM == WII => {
                crate::hollywood::ipc::ipc_write(self, phys, 2, raw);
            }
            HOLLYWOOD_IRQ_BASE..=HOLLYWOOD_IRQ_END if SYSTEM == WII => {
                crate::hollywood::irq::irq_write(self, phys, 2, raw);
            }
            HOLLYWOOD_GPIO_PPC_BASE..=HOLLYWOOD_GPIO_PPC_END if SYSTEM == WII => {
                crate::hollywood::gpio::gpio_write(self, phys, 2, raw);
            }
            HOLLYWOOD_GPIO_ARM_BASE..=HOLLYWOOD_GPIO_ARM_END if SYSTEM == WII => {
                crate::hollywood::gpio::gpio_write(self, phys, 2, raw);
            }
            HOLLYWOOD_COMPAT_BASE..=HOLLYWOOD_COMPAT_END | HOLLYWOOD_PLL_AI_BASE..=HOLLYWOOD_PLL_AI_END
                if SYSTEM == WII =>
            {
                crate::hollywood::compat::compat_write(self, phys, 2, raw);
            }
            HW_HOLLYWOOD_BASE..=HW_HOLLYWOOD_END if SYSTEM == WII => {
                tracing::warn!(
                    device = "HOLLYWOOD",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    val = format!("{raw:08X}"),
                    size = 2,
                    "Unimplemented MMIO write"
                );
                self.mmio.phys_write_u16(phys, val);
            }
            GX_FIFO_BASE..=GX_FIFO_END => {
                let wptr = self.pi.fifo_wptr;
                self.mmio.phys_write_u16(wptr, val);
                self.pi.advance_fifo_wptr(2);
                if self.cp.control.gp_link_enable() {
                    self.gx.mmio_write_u16(&mut self.mmio, self.render_sink.as_mut(), val);
                    self.check_gx_pe_interrupts();
                }
            }
            HW_REG_BASE..=HW_REG_END => {
                tracing::warn!(
                    device = "HW_REG",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    val = format!("{raw:08X}"),
                    size = 2,
                    "Unimplemented MMIO write"
                );
                self.mmio.phys_write_u16(phys, val);
            }
            _ => self.mmio.phys_write_u16(phys, val),
        }
    }

    #[inline(never)]
    fn write_u32_mmio(&mut self, phys: u32, addr: u32, val: u32) {
        match phys {
            CP_BASE..=CP_END => {
                if !crate::flipper::cp::cp_write(self, phys, 4, val) {
                    tracing::warn!(
                        device = "CP",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{val:08X}"),
                        size = 4,
                        "Unimplemented MMIO write"
                    );
                }
            }
            PE_BASE..=PE_END => {
                if !crate::flipper::pe::pe_write(self, phys, 4, val) {
                    tracing::warn!(
                        device = "PE",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{val:08X}"),
                        size = 4,
                        "Unimplemented MMIO write"
                    );
                }
            }
            VI_BASE..=VI_END => {
                if !crate::flipper::vi::vi_write(self, phys, 4, val) {
                    tracing::warn!(
                        device = "VI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{val:08X}"),
                        size = 4,
                        "Unimplemented MMIO write"
                    );
                }
            }
            PI_BASE..=PI_END => {
                if !crate::flipper::pi::pi_write(self, phys, 4, val) {
                    tracing::warn!(
                        device = "PI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{val:08X}"),
                        size = 4,
                        "Unimplemented MMIO write"
                    );
                }
            }
            MI_BASE..=MI_END => {
                if !crate::flipper::mi::mi_write(self, phys, 4, val) {
                    tracing::warn!(
                        device = "MI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{val:08X}"),
                        size = 4,
                        "Unimplemented MMIO write"
                    );
                }
            }
            DSP_BASE..=DSP_END => {
                if !crate::flipper::dsp::dsp_write(self, phys, 4, val) {
                    tracing::warn!(
                        device = "DSP",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{val:08X}"),
                        size = 4,
                        "Unimplemented MMIO write"
                    );
                }
            }
            DI_BASE..=DI_END => {
                if !crate::dvd::di_write(self, phys, 4, val) {
                    tracing::warn!(
                        device = "DI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{val:08X}"),
                        size = 4,
                        "Unimplemented MMIO write"
                    );
                }
            }
            SI_BASE..=SI_END => {
                if !crate::flipper::si::si_write(self, phys, 4, val) {
                    tracing::warn!(
                        device = "SI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{val:08X}"),
                        size = 4,
                        "Unimplemented MMIO write"
                    );
                }
            }
            EXI_BASE..=EXI_END => {
                if !crate::flipper::exi::exi_write(self, phys, 4, val) {
                    tracing::warn!(
                        device = "EXI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{val:08X}"),
                        size = 4,
                        "Unimplemented MMIO write"
                    );
                }
            }
            AI_BASE..=AI_END => {
                if !crate::flipper::ai::ai_write(self, phys, 4, val) {
                    tracing::warn!(
                        device = "AI",
                        addr = format!("{addr:08X}"),
                        phys_addr = format!("{phys:08X}"),
                        val = format!("{val:08X}"),
                        size = 4,
                        "Unimplemented MMIO write"
                    );
                }
            }
            IPC_BASE..=IPC_END if SYSTEM == WII => {
                crate::hollywood::ipc::ipc_write(self, phys, 4, val);
            }
            HOLLYWOOD_IRQ_BASE..=HOLLYWOOD_IRQ_END if SYSTEM == WII => {
                crate::hollywood::irq::irq_write(self, phys, 4, val);
            }
            HOLLYWOOD_GPIO_PPC_BASE..=HOLLYWOOD_GPIO_PPC_END if SYSTEM == WII => {
                crate::hollywood::gpio::gpio_write(self, phys, 4, val);
            }
            HOLLYWOOD_GPIO_ARM_BASE..=HOLLYWOOD_GPIO_ARM_END if SYSTEM == WII => {
                crate::hollywood::gpio::gpio_write(self, phys, 4, val);
            }
            HOLLYWOOD_COMPAT_BASE..=HOLLYWOOD_COMPAT_END | HOLLYWOOD_PLL_AI_BASE..=HOLLYWOOD_PLL_AI_END
                if SYSTEM == WII =>
            {
                crate::hollywood::compat::compat_write(self, phys, 4, val);
            }
            HW_HOLLYWOOD_BASE..=HW_HOLLYWOOD_END if SYSTEM == WII => {
                tracing::warn!(
                    device = "HOLLYWOOD",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    val = format!("{val:08X}"),
                    size = 4,
                    "Unimplemented MMIO write"
                );
                self.mmio.phys_write_u32(phys, val);
            }
            GX_FIFO_BASE..=GX_FIFO_END => {
                let wptr = self.pi.fifo_wptr;
                self.mmio.phys_write_u32(wptr, val);
                self.pi.advance_fifo_wptr(4);
                if self.cp.control.gp_link_enable() {
                    self.gx.mmio_write_u32(&mut self.mmio, self.render_sink.as_mut(), val);
                    self.check_gx_pe_interrupts();
                }
            }
            HW_REG_BASE..=HW_REG_END => {
                tracing::warn!(
                    device = "HW_REG",
                    addr = format!("{addr:08X}"),
                    phys_addr = format!("{phys:08X}"),
                    val = format!("{val:08X}"),
                    size = 4,
                    "Unimplemented MMIO write"
                );
                self.mmio.phys_write_u32(phys, val);
            }
            _ => self.mmio.phys_write_u32(phys, val),
        }
    }
}
