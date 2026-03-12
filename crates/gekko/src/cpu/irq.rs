use crate::{cpu::spr::Srr0, gekko::Gekko, mmio::Mmio};

// Exception vectors
#[rustfmt::skip] pub const IRQ_SYSTEM_RESET: u32         = Mmio::virt_to_phys(0x8000_0100);
#[rustfmt::skip] pub const IRQ_MACHINE_CHECK: u32        = Mmio::virt_to_phys(0x8000_0200);
#[rustfmt::skip] pub const IRQ_DSI: u32                  = Mmio::virt_to_phys(0x8000_0300);
#[rustfmt::skip] pub const IRQ_ISI: u32                  = Mmio::virt_to_phys(0x8000_0400);
#[rustfmt::skip] pub const IRQ_EXTERNAL: u32             = Mmio::virt_to_phys(0x8000_0500); // traditional IRQ
#[rustfmt::skip] pub const IRQ_ALIGNMENT: u32            = Mmio::virt_to_phys(0x8000_0600);
#[rustfmt::skip] pub const IRQ_PROGRAM: u32              = Mmio::virt_to_phys(0x8000_0700);
#[rustfmt::skip] pub const IRQ_FP_UNAVAILABLE: u32       = Mmio::virt_to_phys(0x8000_0800);
#[rustfmt::skip] pub const IRQ_DECREMENTER: u32          = Mmio::virt_to_phys(0x8000_0900);
#[rustfmt::skip] pub const IRQ_SYSTEM_CALL: u32          = Mmio::virt_to_phys(0x8000_0C00);
#[rustfmt::skip] pub const IRQ_TRACE: u32                = Mmio::virt_to_phys(0x8000_0D00);
#[rustfmt::skip] pub const IRQ_PERFORMANCE_MONITOR: u32  = Mmio::virt_to_phys(0x8000_0F00);
#[rustfmt::skip] pub const IRQ_IABR: u32                 = Mmio::virt_to_phys(0x8000_1300);
#[rustfmt::skip] pub const IRQ_THERMAL: u32              = Mmio::virt_to_phys(0x8000_1400);

impl Gekko {
    pub fn cause_external_interrupt(&mut self) {
        let base: u32 = if self.cpu.msr.exception_prefix() {
            0xFFF0_0000
        } else {
            0
        };

        // Table 4-7. System Reset Exception—Register Settings
        self.cpu.spr.srr0 = Srr0::from(self.cpu.pc);
        self.cpu.spr.srr1 = chapa::extract_bits!(self.cpu.msr; 0, 5..=9, 16..=31).raw();

        self.cpu.msr = self
            .cpu
            .msr
            .with_pow(false)
            .with_fp(false)
            .with_be(false)
            .with_dr(false)
            .with_fe1(false)
            .with_pm(false)
            .with_ee(false)
            .with_fe0(false)
            .with_ri(false)
            .with_pr(false)
            .with_se(false)
            .with_ir(false)
            .with_le(self.cpu.msr.ile());

        self.cpu.pc = base | IRQ_EXTERNAL;

        tracing::debug!(addr = format!("{:08X}", self.cpu.pc), "IRQ triggered");
    }
}
