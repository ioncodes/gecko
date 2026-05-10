use std::sync::Arc;

use crate::state::{RunMode, Shared};

pub fn run(shared: Arc<Shared>) {
    loop {
        let mut s = shared.state.lock().unwrap();
        loop {
            if s.backend.is_none() || s.run_mode == RunMode::Paused {
                s = shared
                    .cv
                    .wait_while(s, |s| s.backend.is_none() || s.run_mode == RunMode::Paused)
                    .unwrap();
                continue;
            }
            break;
        }

        if let Some(backend) = s.backend.as_mut() {
            backend.run_until_vsync();
        }
        drop(s);
    }
}
