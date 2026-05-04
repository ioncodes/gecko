use crate::scheduler::TIMEBASE_DIVISOR;
use crate::system::{System, SystemId};

const DEC_INTERRUPT_BIT: u32 = 0x8000_0000;

pub struct Decrementer {
    start_cycle: u64,
    start_value: u32,
    interrupt_pending: bool,
}

impl Default for Decrementer {
    fn default() -> Self {
        Self {
            start_cycle: 0,
            start_value: u32::MAX,
            interrupt_pending: false,
        }
    }
}

impl Decrementer {
    pub fn read(&self, cycles: u64) -> u32 {
        let elapsed_ticks = cycles.saturating_sub(self.start_cycle) / TIMEBASE_DIVISOR;
        self.start_value.wrapping_sub(elapsed_ticks as u32)
    }

    pub fn write(&mut self, cycles: u64, value: u32) {
        let old_value = self.read(cycles);
        self.start_cycle = cycles;
        self.start_value = value;

        if old_value & DEC_INTERRUPT_BIT == 0 && value & DEC_INTERRUPT_BIT != 0 {
            self.interrupt_pending = true;
        }
    }

    pub fn underflow(&mut self, cycles: u64) {
        self.start_cycle = cycles;
        self.start_value = u32::MAX;
        self.interrupt_pending = true;
    }

    pub fn interrupt_pending(&self) -> bool {
        self.interrupt_pending
    }

    pub fn clear_interrupt(&mut self) {
        self.interrupt_pending = false;
    }
}

pub fn cycles_until_underflow(value: u32) -> u64 {
    value as u64 * TIMEBASE_DIVISOR
}

pub fn underflow_handler<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    sys.gekko.dec.underflow(sys.scheduler.cycles);
    sys.gekko.spr.dec = u32::MAX;
    sys.scheduler
        .schedule_in(cycles_until_underflow(u32::MAX), self::underflow_handler);
    tracing::debug!(
        cycles = sys.scheduler.cycles,
        ee = sys.gekko.msr.external_interrupt_enable(),
        pi_pending = sys.pi.interrupt_pending(),
        pc = format!("{:08X}", sys.gekko.pc),
        "decrementer underflow"
    );
}
