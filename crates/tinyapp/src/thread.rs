use crossbeam_channel::Sender;
use gecko::{
    flipper::{si::pad::PadStatus, vi::regs::RefreshRate},
    gamecube::GameCube,
};
use std::sync::{Arc, Mutex};
use winit::event_loop::EventLoopProxy;

pub struct FrameMessage {
    pub native_hz: f64,
}

pub fn emu_thread(
    mut emulator: GameCube,
    frame_tx: Sender<FrameMessage>,
    input: Arc<Mutex<PadStatus>>,
    proxy: EventLoopProxy<()>,
) {
    loop {
        *emulator.primary_controller_mut() = *input.lock().unwrap();
        emulator.run_until_vsync();

        let native_hz = match emulator.vi.dcr.video_format().refresh_rate() {
            RefreshRate::Hz60 => 60.0,
            RefreshRate::Hz50 => 50.0,
        };

        if frame_tx.send(FrameMessage { native_hz }).is_err() {
            break;
        }
        let _ = proxy.send_event(());
    }
}
