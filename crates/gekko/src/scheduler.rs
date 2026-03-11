use std::cmp::Reverse;
use std::collections::BinaryHeap;

pub const CYCLES_PER_VSYNC: u64 = 486_000_000 / 60; // TODO: fix

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventKind {
    VSync,
}

pub struct Scheduler {
    pub cycles: u64,
    events: BinaryHeap<Reverse<(u64, EventKind)>>,
}

impl Scheduler {
    pub fn new() -> Self {
        let mut s = Scheduler {
            cycles: 0,
            events: BinaryHeap::new(),
        };
        s.schedule_at(CYCLES_PER_VSYNC, EventKind::VSync);
        s
    }

    pub fn schedule_at(&mut self, deadline: u64, kind: EventKind) {
        self.events.push(Reverse((deadline, kind)));
    }

    pub fn poll(&mut self) -> Option<EventKind> {
        if self.events.peek().map_or(false, |Reverse((d, _))| self.cycles >= *d) {
            Some(self.events.pop().unwrap().0.1)
        } else {
            None
        }
    }
}
