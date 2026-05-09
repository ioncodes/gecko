pub mod audio;
pub mod common;
pub mod dvd;
pub mod flipper;
#[cfg(feature = "fps-counter")]
pub mod fps;
pub mod gamecube;
pub mod gekko;
pub mod hollywood;
pub mod host;
pub mod input;
pub mod ipl;
pub mod jit_cache;
pub mod mmio;
#[cfg(any(feature = "jit-stats", feature = "profile", feature = "gx-stats"))]
pub mod profile;
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
