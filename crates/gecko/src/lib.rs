pub mod audio;
pub mod common;
pub mod dvd;
pub mod flipper;
pub mod gamecube;
pub mod gekko;
pub mod hollywood;
pub mod host;
pub mod input;
pub mod ipl;
pub mod mmio;
pub mod scheduler;
pub mod starlet;
pub mod system;
pub mod wii;

pub use gamecube::GameCube;
pub use input::HostInput;
pub use system::{GC, System, SystemId, WII};
pub use wii::Wii;

#[cfg(feature = "hooks")]
pub mod hooks;
