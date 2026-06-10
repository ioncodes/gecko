use gecko::HostInput;
use gecko::flipper::gx::recorder::FifoRecorder;
use gecko::system::{System, SystemId};
use spin_sleep::SpinSleeper;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn emu_thread<const SYSTEM: SystemId>(
    mut emulator: System<SYSTEM>,
    input: Arc<Mutex<HostInput>>,
    game_id: Option<String>,
    throttle: bool,
    start_gate: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    fifo_record: Option<String>,
) {
    let sleeper = SpinSleeper::default();
    let throttle_step = Duration::from_micros(5);

    while !start_gate.load(Ordering::Acquire) {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        sleeper.sleep(Duration::from_millis(10));
    }

    if fifo_record.is_some() {
        tracing::info!("FIFO recorder started, recording until shutdown");
        emulator.gx.recorder = Some(Box::new(FifoRecorder::new()));
    }

    emulator.set_input_sink(Box::new(input.clone()));

    while !shutdown.load(Ordering::Relaxed) {
        while throttle && emulator.audio_sink.should_throttle() {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            sleeper.sleep(throttle_step);
        }

        let input = *input.lock().unwrap();
        emulator.apply_host_input(&input);
        emulator.run_until_vsync();
    }

    if let Some(path) = fifo_record
        && let Some(rec) = emulator.gx.recorder.take()
    {
        let file = rec.into_file();
        let frame_count = file.frames.len();
        if frame_count == 0 {
            tracing::warn!("nothing to save");
        } else {
            match file.save(std::path::Path::new(&path)) {
                Ok(()) => tracing::info!(frames = frame_count, path = path.as_str(), "FIFO dump saved"),
                Err(err) => tracing::error!(%err, "FIFO dump save failed"),
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
