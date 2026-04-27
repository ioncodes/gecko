pub mod common;
pub mod cpu;
pub mod dvd;
pub mod flipper;
pub mod gamecube;
pub mod host;
pub mod idle;
pub mod ipl;
pub mod mmio;
pub mod scheduler;
pub mod system;
pub mod wii;

pub use gamecube::GameCube;
pub use system::{GC, System, SystemId, WII};
pub use wii::Wii;

#[cfg(feature = "hooks")]
pub mod hooks;
