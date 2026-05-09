use gecko::HostInput;
use gecko::system::{System, SystemId};
use std::sync::{Arc, Mutex};
use winit::event_loop::EventLoopProxy;

pub fn emu_thread<const SYSTEM: SystemId>(
    mut emulator: System<SYSTEM>,
    input: Arc<Mutex<HostInput>>,
    proxy: EventLoopProxy<()>,
    game_id: Option<String>,
) {
    loop {
        let input = *input.lock().unwrap();
        emulator.apply_host_input(&input);
        emulator.run_until_vsync();

        if proxy.send_event(()).is_err() {
            break;
        }
    }

    if let Some(game_id) = game_id.as_deref() {
        match emulator.save_jit_cache(game_id) {
            Ok((ppc, dsp)) => tracing::info!(ppc_blocks = ppc, dsp_blocks = dsp, "saved JIT cache"),
            Err(err) => tracing::warn!(?err, "failed to save JIT cache"),
        }
    }
}
