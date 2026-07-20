use gecko::HostInput;
use gecko::system::{System, SystemId};
use spin_sleep::SpinSleeper;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn run<const SYSTEM: SystemId>(
    mut emulator: System<SYSTEM>,
    input: Arc<Mutex<HostInput>>,
    input_config: hostinput::InputConfig,
    game_id: Option<String>,
    throttle: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    savestate_path: std::path::PathBuf,
    save_state: Arc<AtomicBool>,
    load_state: Arc<AtomicBool>,
) {
    let sleeper = SpinSleeper::default();
    let throttle_step = Duration::from_micros(5);
    let pause_step = Duration::from_millis(10);

    emulator.set_input_sink(Box::new(hostinput::InputManager::new(
        SYSTEM,
        &input_config,
        input.clone(),
    )));

    while !shutdown.load(Ordering::Relaxed) {
        if paused.load(Ordering::Relaxed) {
            sleeper.sleep(pause_step);
            continue;
        }

        while throttle.load(Ordering::Relaxed) && emulator.audio_sink.should_throttle() {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            sleeper.sleep(throttle_step);
        }

        emulator.run_until_vsync();

        if save_state.swap(false, Ordering::Relaxed) {
            match emulator.save_state_to_file(&savestate_path) {
                Ok(()) => tracing::info!(path = %savestate_path.display(), "savestate saved"),
                Err(err) => tracing::error!(%err, "savestate save failed"),
            }
        }

        if load_state.swap(false, Ordering::Relaxed) {
            match emulator.load_state_from_file(&savestate_path) {
                Ok(()) => tracing::info!(path = %savestate_path.display(), "savestate loaded"),
                Err(err) => tracing::error!(%err, "savestate load failed"),
            }
        }
    }

    if let Some(game_id) = game_id.as_deref() {
        match emulator.save_jit_cache(game_id) {
            Ok((ppc, dsp, vtx)) => {
                tracing::info!(ppc_blocks = ppc, dsp_blocks = dsp, vtx_keys = vtx, "saved JIT cache")
            }
            Err(err) => tracing::warn!(?err, "failed to save JIT cache"),
        }
    }
}
