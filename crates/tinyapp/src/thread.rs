use crossbeam_channel::Sender;
use gecko::{
    flipper::{gx::draw::DrawCommands, si::pad::PadStatus, vi::regs::RefreshRate},
    gamecube::GameCube,
};
use std::sync::{Arc, Mutex};
use winit::event_loop::EventLoopProxy;

pub enum FrameData {
    Gx { commands: DrawCommands, ram: Vec<u8> },
    Xfb { pixels: Vec<u32> },
}

pub struct FrameMessage {
    pub data: FrameData,
    pub width: u32,
    pub height: u32,
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

        let (w, h) = emulator.frame_size();
        let (width, height) = (w as u32, h as u32);

        let data = if !emulator.gx.draw_commands.commands.is_empty() {
            FrameData::Gx {
                commands: std::mem::take(&mut emulator.gx.draw_commands),
                ram: emulator.mmio.ram.clone(),
            }
        } else {
            FrameData::Xfb {
                pixels: emulator.render_xfb(),
            }
        };

        if frame_tx
            .send(FrameMessage {
                data,
                width,
                height,
                native_hz,
            })
            .is_err()
        {
            break;
        }
        let _ = proxy.send_event(());
    }
}
