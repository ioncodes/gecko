use std::collections::VecDeque;

use crate::flipper::vi::regs::RefreshRate;
use crate::gamecube::GameCube;

pub const TIMEBASE_DIVISOR: u64 = 12;
pub const CPU_CYCLES_PER_DSP_TICK: u64 = 6; // ~486MHz CPU / ~81MHz DSP
pub const DSP_BATCH_SIZE: u64 = 1024;

pub type ScheduledFn = fn(&mut GameCube);

#[derive(Clone, Copy)]
struct ScheduledEvent {
    deadline: u64,
    f: ScheduledFn,
}

pub struct Scheduler {
    pub cycles: u64,
    next_deadline: u64,
    timebase_offset: i64,
    events: VecDeque<ScheduledEvent>,
}

impl Scheduler {
    pub fn new() -> Self {
        let mut s = Scheduler {
            cycles: 0,
            next_deadline: u64::MAX,
            timebase_offset: 0,
            events: VecDeque::with_capacity(8),
        };
        let initial_refresh_rate = RefreshRate::Hz60; // TODO: Detect IPL and schedule accordingly
        s.schedule_at(initial_refresh_rate.cycles_per_frame(), crate::scheduler::vsync_handler);
        s.schedule_at(
            CPU_CYCLES_PER_DSP_TICK * DSP_BATCH_SIZE,
            crate::scheduler::dsp_batch_handler,
        );
        s
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
    pub fn schedule_at(&mut self, deadline: u64, f: ScheduledFn) {
        let pos = self.events.partition_point(|e| e.deadline <= deadline);
        self.events.insert(pos, ScheduledEvent { deadline, f });
        self.next_deadline = self.next_deadline.min(deadline);
    }

    pub fn schedule_in(&mut self, delay: u64, f: ScheduledFn) {
        let deadline = self.cycles + delay;
        self.schedule_at(deadline, f);
    }

    #[inline(always)]
    pub fn poll(&mut self) -> Option<ScheduledFn> {
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

/// Reschedules itself every frame.
pub fn vsync_handler(gc: &mut GameCube) {
    gc.vsync_pending = true;
    let rate = gc.vi.dcr.video_format().refresh_rate();
    gc.scheduler
        .schedule_in(rate.cycles_per_frame(), crate::scheduler::vsync_handler);
}

/// Reschedules itself every DSP batch.
pub fn dsp_batch_handler(gc: &mut GameCube) {
    gc.execute_dsp_batch();
    gc.scheduler.schedule_in(
        crate::scheduler::CPU_CYCLES_PER_DSP_TICK * crate::scheduler::DSP_BATCH_SIZE,
        crate::scheduler::dsp_batch_handler,
    );
}
