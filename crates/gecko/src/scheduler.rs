use std::cmp::Reverse;
use std::collections::BinaryHeap;

pub const CYCLES_PER_VSYNC: u64 = 486_000_000 / 60; // TODO: fix
pub const TIMEBASE_DIVISOR: u64 = 12;
pub const DSP_TICKS_PER_CPU_CYCLE: u64 = 6; // ~486MHz CPU / ~81MHz DSP

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventKind {
    VSync,
    ViHalfLine,
    DiTransferComplete,
    DspTick,
}

pub struct Scheduler {
    pub cycles: u64,
    timebase_offset: i64,
    events: BinaryHeap<Reverse<(u64, EventKind)>>,
}

impl Scheduler {
    pub fn new() -> Self {
        let mut s = Scheduler {
            cycles: 0,
            timebase_offset: 0,
            events: BinaryHeap::new(),
        };
        s.schedule_at(CYCLES_PER_VSYNC, EventKind::VSync);
        s.schedule_at(DSP_TICKS_PER_CPU_CYCLE, EventKind::DspTick);
        s
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

    pub fn schedule_at(&mut self, deadline: u64, kind: EventKind) {
        self.events.push(Reverse((deadline, kind)));
    }

    pub fn schedule_in(&mut self, delay: u64, kind: EventKind) {
        let deadline = self.cycles + delay;
        self.schedule_at(deadline, kind);
    }

    pub fn poll(&mut self) -> Option<EventKind> {
        if self.events.peek().map_or(false, |Reverse((d, _))| self.cycles >= *d) {
            Some(self.events.pop().unwrap().0.1)
        } else {
            None
        }
    }

    pub fn next_event_deadline(&self) -> Option<u64> {
        self.events.peek().map(|Reverse((d, _))| *d)
    }
}
