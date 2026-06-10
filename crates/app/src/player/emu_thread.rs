use gecko::HostInput;
use gecko::system::{System, SystemId};
use spin_sleep::SpinSleeper;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn run<const SYSTEM: SystemId>(
    mut emulator: System<SYSTEM>,
    input: Arc<Mutex<HostInput>>,
    game_id: Option<String>,
    throttle: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
) {
    let sleeper = SpinSleeper::default();
    let throttle_step = Duration::from_micros(5);

    emulator.set_input_sink(Box::new(input.clone()));

    while !shutdown.load(Ordering::Relaxed) {
        while throttle.load(Ordering::Relaxed) && emulator.audio_sink.should_throttle() {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            sleeper.sleep(throttle_step);
        }

        let input = *input.lock().unwrap();
        emulator.apply_host_input(&input);
        emulator.run_until_vsync();
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
