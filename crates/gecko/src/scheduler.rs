use std::collections::VecDeque;

use crate::flipper::vi::regs::RefreshRate;
use crate::system::{self, System, SystemId};

pub const TIMEBASE_DIVISOR: u64 = 12;
pub const DSP_BATCH_SIZE: u64 = 1024;

#[inline(always)]
#[rustfmt::skip]
pub const fn cpu_clock(system: SystemId) -> u64 {
    match system {
        system::WII => 729_000_000,
        system::GC  => 486_000_000,
        _ => unreachable!(),
    }
}

#[inline(always)]
#[rustfmt::skip]
pub const fn cpu_cycles_per_dsp_tick(system: SystemId) -> u64 {
    match system {
        system::WII => 9, // 729 MHz / 81 MHz
        system::GC  => 6, // 486 MHz / 81 MHz
        _ => unreachable!(),
    }
}

#[inline(always)]
pub const fn microseconds_to_cycles(system: SystemId, us: u64) -> u64 {
    us * (self::cpu_clock(system) / 1_000_000)
}

pub type ScheduledFn<const SYSTEM: SystemId> = fn(&mut System<SYSTEM>);

#[derive(Clone, Copy)]
struct ScheduledEvent<const SYSTEM: SystemId> {
    deadline: u64,
    f: ScheduledFn<SYSTEM>,
}

pub struct Scheduler<const SYSTEM: SystemId> {
    pub cycles: u64,
    next_deadline: u64,
    timebase_offset: i64,
    events: VecDeque<ScheduledEvent<SYSTEM>>,
}

impl<const SYSTEM: SystemId> Scheduler<SYSTEM> {
    pub fn empty() -> Self {
        Scheduler {
            cycles: 0,
            next_deadline: u64::MAX,
            timebase_offset: 0,
            events: VecDeque::with_capacity(8),
        }
    }

    #[inline(always)]
    pub fn refresh_deadline(&mut self) {
        self.next_deadline = self.events.front().map_or(u64::MAX, |e| e.deadline);
    }

    pub fn timebase(&self) -> u64 {
        ((self.cycles / TIMEBASE_DIVISOR) as i64 + self.timebase_offset) as u64
    }

    pub fn set_timebase_lower(&mut self, val: u32) {
        let current = self.timebase();
        let new_tb = (current & 0xFFFF_FFFF_0000_0000) | val as u64;
        self.timebase_offset = new_tb as i64 - (self.cycles / TIMEBASE_DIVISOR) as i64;
    }

    pub fn set_timebase_upper(&mut self, val: u32) {
        let current = self.timebase();
        let new_tb = ((val as u64) << 32) | (current & 0xFFFF_FFFF);
        self.timebase_offset = new_tb as i64 - (self.cycles / TIMEBASE_DIVISOR) as i64;
    }

    pub fn timebase_lower(&self) -> u32 {
        self.timebase() as u32
    }

    pub fn timebase_upper(&self) -> u32 {
        (self.timebase() >> 32) as u32
    }

    /// Insert an event keeping the deque sorted by deadline (earliest first).
    pub fn schedule_at(&mut self, deadline: u64, f: ScheduledFn<SYSTEM>) {
        let pos = self.events.partition_point(|e| e.deadline <= deadline);
        self.events.insert(pos, ScheduledEvent { deadline, f });
        self.next_deadline = self.next_deadline.min(deadline);
    }

    pub fn cancel(&mut self, f: ScheduledFn<SYSTEM>) {
        self.events.retain(|e| !std::ptr::fn_addr_eq(e.f, f));
        self.refresh_deadline();
    }

    pub fn schedule_in(&mut self, delay: u64, f: ScheduledFn<SYSTEM>) {
        let deadline = self.cycles + delay;
        self.schedule_at(deadline, f);
    }

    #[inline(always)]
    pub fn poll(&mut self) -> Option<ScheduledFn<SYSTEM>> {
        let front = self.events.front()?;
        if self.cycles >= front.deadline {
            let f = self.events.pop_front().unwrap().f;
            self.refresh_deadline();
            Some(f)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn next_deadline(&self) -> u64 {
        self.next_deadline
    }
}

impl Scheduler<{ crate::system::GC }> {
    pub fn new_gamecube() -> Self {
        Self::with_default_events()
    }
}

impl Scheduler<{ crate::system::WII }> {
    pub fn new_wii() -> Self {
        Self::with_default_events()
    }
}

impl<const SYSTEM: SystemId> Scheduler<SYSTEM> {
    fn with_default_events() -> Self {
        let mut s = Self::empty();
        let initial_refresh_rate = RefreshRate::Hz60; // TODO: Detect IPL and schedule accordingly
        s.schedule_at(
            initial_refresh_rate.cycles_per_frame(SYSTEM),
            self::vsync_handler::<SYSTEM>,
        );
        s.schedule_at(
            self::cpu_cycles_per_dsp_tick(SYSTEM) * self::DSP_BATCH_SIZE,
            self::dsp_batch_handler::<SYSTEM>,
        );
        s.schedule_at(
            crate::gekko::dec::cycles_until_underflow(u32::MAX),
            crate::gekko::dec::underflow_handler::<SYSTEM>,
        );
        s
    }
}

/// Reschedules itself every frame.
pub fn vsync_handler<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    sys.vsync_pending = true;
    let rate = sys.vi.dcr.video_format().refresh_rate();
    sys.scheduler
        .schedule_in(rate.cycles_per_frame(SYSTEM), self::vsync_handler::<SYSTEM>);
}

/// Reschedules itself every DSP batch.
pub fn dsp_batch_handler<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    sys.execute_dsp_batch();
    sys.scheduler.schedule_in(
        self::cpu_cycles_per_dsp_tick(SYSTEM) * self::DSP_BATCH_SIZE,
        self::dsp_batch_handler::<SYSTEM>,
    );
}
