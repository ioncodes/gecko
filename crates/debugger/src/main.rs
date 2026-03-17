use std::env;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

use gekko::gekko::Gekko;
use image::Dol;

use crate::debugger::DebuggerUi;
use crate::render::RenderState;

mod debugger;
mod render;
mod windows;

struct App {
    emulator: Gekko,
    ui: DebuggerUi,
    window: Option<Arc<Window>>,
    state: Option<RenderState>,
    present_mode: wgpu::PresentMode,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("Gekko"))
                .unwrap(),
        );

        let state = RenderState::new(window.clone(), &self.emulator, self.present_mode);
        window.request_redraw();
        self.window = Some(window);
        self.state = Some(state);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let (Some(state), Some(window)) = (&mut self.state, &self.window) {
            let _ = state.egui_winit.on_window_event(window, &event);
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(state) = &mut self.state {
                    state.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                if let (Some(state), Some(window)) = (&mut self.state, &self.window) {
                    state.render(&mut self.emulator, &mut self.ui, window);
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: {} <path/to/game.dol> [--immediate] [--idle-skip]", args[0]);
        std::process::exit(1);
    }

    let present_mode = args
        .iter()
        .any(|a| a == "--immediate")
        .then_some(wgpu::PresentMode::Immediate)
        .unwrap_or(wgpu::PresentMode::Fifo);
    let idle_skip = args.iter().any(|a| a == "--idle-skip");

    let rom_data = std::fs::read(&args[1]).expect("failed to read ROM");
    let dol = Dol::parse(rom_data);
    let emulator = Gekko::with_image(&dol, idle_skip);

    let event_loop = EventLoop::new().unwrap();
    let mut app = App {
        emulator,
        ui: DebuggerUi::default(),
        window: None,
        state: None,
        present_mode,
    };
    event_loop.run_app(&mut app).unwrap();
}
