use std::env;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

use gekko::flipper::si::pad::{self, PadStatus, STICK_CENTER};
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
        let mut egui_consumed = false;
        if let (Some(state), Some(window)) = (&mut self.state, &self.window) {
            let response = state.egui_winit.on_window_event(window, &event);
            egui_consumed = response.consumed;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(state) = &mut self.state {
                    state.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if !egui_consumed => {
                let pressed = event.state.is_pressed();
                if let PhysicalKey::Code(key) = event.physical_key {
                    update_pad(self.emulator.primary_controller_mut(), key, pressed);
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

    let present_mode = args
        .iter()
        .any(|a| a == "--immediate")
        .then_some(wgpu::PresentMode::Immediate)
        .unwrap_or(wgpu::PresentMode::Fifo);
    let idle_skip = args.iter().any(|a| a == "--idle-skip");

    let ipl_path = args.iter().position(|a| a == "--ipl").map(|i| &args[i + 1]);
    let rom_path = args
        .iter()
        .position(|a| a == "--rom")
        .map(|i| &args[i + 1])
        .or_else(|| args.get(1).filter(|a| !a.starts_with("--")));

    let mut emulator = if let Some(ipl) = ipl_path {
        let ipl_data = std::fs::read(ipl).expect("failed to read IPL");
        Gekko::with_ipl(&ipl_data, idle_skip)
    } else if let Some(rom) = rom_path {
        let rom_data = std::fs::read(rom).expect("failed to read ROM");
        let dol = Dol::parse(rom_data);
        Gekko::with_image(&dol, idle_skip)
    } else {
        eprintln!(
            "usage: {} <path/to/game.dol> | --ipl <path> | --rom <path> [--immediate] [--idle-skip]",
            args[0]
        );
        std::process::exit(1);
    };

    emulator.add_primary_controller(PadStatus {
        connected: true,
        ..PadStatus::default()
    });

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

fn update_pad(pad: &mut PadStatus, key: KeyCode, pressed: bool) {
    let set_button = |buttons: &mut u16, mask: u16, on: bool| {
        if on {
            *buttons |= mask;
        } else {
            *buttons &= !mask;
        }
    };

    match key {
        // Analog stick (digital, full deflection)
        KeyCode::ArrowUp => pad.stick_y = if pressed { 255 } else { STICK_CENTER },
        KeyCode::ArrowDown => pad.stick_y = if pressed { 0 } else { STICK_CENTER },
        KeyCode::ArrowLeft => pad.stick_x = if pressed { 0 } else { STICK_CENTER },
        KeyCode::ArrowRight => pad.stick_x = if pressed { 255 } else { STICK_CENTER },

        // Face buttons
        KeyCode::KeyX => set_button(&mut pad.buttons, pad::A, pressed),
        KeyCode::KeyZ => set_button(&mut pad.buttons, pad::B, pressed),
        KeyCode::KeyC => set_button(&mut pad.buttons, pad::X, pressed),
        KeyCode::KeyV => set_button(&mut pad.buttons, pad::Y, pressed),
        KeyCode::Enter => set_button(&mut pad.buttons, pad::START, pressed),

        // Triggers
        KeyCode::KeyA => {
            set_button(&mut pad.buttons, pad::L, pressed);
            pad.trigger_left = if pressed { 255 } else { 0 };
        }
        KeyCode::KeyS => {
            set_button(&mut pad.buttons, pad::R, pressed);
            pad.trigger_right = if pressed { 255 } else { 0 };
        }
        KeyCode::KeyD => set_button(&mut pad.buttons, pad::Z, pressed),

        // D-pad
        KeyCode::KeyI => set_button(&mut pad.buttons, pad::DPAD_UP, pressed),
        KeyCode::KeyK => set_button(&mut pad.buttons, pad::DPAD_DOWN, pressed),
        KeyCode::KeyJ => set_button(&mut pad.buttons, pad::DPAD_LEFT, pressed),
        KeyCode::KeyL => set_button(&mut pad.buttons, pad::DPAD_RIGHT, pressed),
        _ => {}
    }
}
