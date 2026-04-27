use crate::scheduler::Scheduler;
use crate::system::{System, WII};

pub type Wii = System<{ WII }>;

impl Wii {
    pub fn new(entrypoint: u32) -> Self {
        Self::with_scheduler(entrypoint, Scheduler::new_wii())
    }
}
