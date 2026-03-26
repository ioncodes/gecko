use std::env;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

use gecko::flipper::si::pad::{self, PadStatus, STICK_CENTER};
use gecko::gamecube::GameCube;
use image::{Dol, dvd::Dvd};

use crate::debugger::DebuggerUi;
use crate::render::RenderState;

mod debugger;
mod render;
mod windows;

struct App {
    emulator: GameCube,
    ui: DebuggerUi,
    window: Option<Arc<Window>>,
    state: Option<RenderState>,
    present_mode: wgpu::PresentMode,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("Gecko"))
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

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
    let iso_path = args.iter().position(|a| a == "--iso").map(|i| &args[i + 1]);
    let script_path = args.iter().position(|a| a == "--script").map(|i| &args[i + 1]);

    let mut emulator = if let Some(ipl) = ipl_path {
        let ipl_data = std::fs::read(ipl).expect("failed to read IPL");
        GameCube::with_ipl(&ipl_data, idle_skip)
    } else if let Some(rom) = rom_path {
        let rom_data = std::fs::read(rom).expect("failed to read ROM");
        let dol = Dol::parse(rom_data);
        GameCube::with_image(&dol, idle_skip)
    } else {
        eprintln!(
            "usage: {} <path/to/game.dol> | --ipl <path> | --rom <path> [--iso <path>] [--immediate] [--idle-skip]",
            args[0]
        );
        std::process::exit(1);
    };

    if let Some(iso) = iso_path {
        if ipl_path.is_none() {
            eprintln!("--iso requires --ipl");
            std::process::exit(1);
        }

        let iso_data = std::fs::read(iso).expect("failed to read ISO");
        let dvd = Dvd::parse(iso_data);
        emulator.insert_dvd(dvd);
    }

    if let Some(path) = script_path {
        let host = scripting::LuaScriptHost::from_file(path).expect("failed to load script");
        emulator.set_script_host(Box::new(host));
    }

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
