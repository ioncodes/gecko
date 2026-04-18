use backend_wgpu::capture::CaptureRequest;
use clap::Parser;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use gecko::flipper::si::pad::{self, PadStatus, STICK_CENTER};
use gecko::gamecube::GameCube;
use image::Dol;
use image::dvd::Dvd;

use crate::debugger::DebuggerUi;
use crate::render::RenderState;

mod debugger;
mod render;

struct App {
    emulator: GameCube,
    ui: DebuggerUi,
    window: Option<Arc<Window>>,
    state: Option<RenderState>,
    present_mode: wgpu::PresentMode,
    init: Option<AppInit>,
}

struct AppInit {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: backend_wgpu::sink::Renderer,
    surface_format: wgpu::TextureFormat,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("Gecko"))
                .unwrap(),
        );

        let Some(init) = self.init.take() else {
            self.window = Some(window);
            return;
        };

        let surface = init.instance.create_surface(window.clone()).unwrap();
        let size = window.inner_size();
        let surface_caps = surface.get_capabilities(&init.adapter);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            format: init.surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: self.present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&init.device, &surface_config);

        let state = RenderState::new(
            window.clone(),
            init.device,
            init.queue,
            surface,
            surface_config,
            init.renderer,
            self.present_mode,
        );
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
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state.is_pressed();
                if let PhysicalKey::Code(key) = event.physical_key {
                    if pressed && !event.repeat {
                        if let Some(state) = &mut self.state {
                            match key {
                                KeyCode::F11 => state.request_screenshot(CaptureRequest::FullWindow),
                                KeyCode::F12 => state.request_screenshot(CaptureRequest::GameOnly),
                                _ => {}
                            }
                        }
                    }

                    if !egui_consumed {
                        update_pad(self.emulator.primary_controller_mut(), key, pressed);
                    }
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

#[derive(Parser)]
#[command(about = "GameCube debugger")]
struct Args {
    /// Path to the DOL file
    #[arg(long)]
    dol: Option<String>,

    /// Path to an IPL file
    #[arg(long)]
    ipl: Option<String>,

    /// Patch the IPL to skip directly to disc boot
    #[arg(long)]
    skip_ipl: bool,

    /// Boot from ISO using HLE IPL (requires --iso)
    #[arg(long)]
    ipl_hle: bool,

    /// Path to a GameCube ISO file
    #[arg(long)]
    iso: Option<String>,

    /// Use immediate present mode (no vsync)
    #[arg(long)]
    immediate: bool,

    /// Path to a DSP IROM binary
    #[arg(long)]
    dsp: Option<String>,

    /// Path to a symbol file (ELF, IDA .idb, or .i64)
    #[arg(long)]
    symbols: Option<String>,

    /// Path to a Lua script for scripting hooks
    #[arg(long)]
    script: Option<String>,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .without_time()
        .init();

    let args = Args::parse();

    let present_mode = if args.immediate {
        wgpu::PresentMode::Immediate
    } else {
        wgpu::PresentMode::Fifo
    };

    let mut emulator = if args.ipl_hle {
        let Some(ref iso) = args.iso else {
            eprintln!("--ipl-hle requires --iso");
            std::process::exit(1);
        };
        let iso_data = std::fs::read(iso).expect("failed to read ISO");
        let dvd = Dvd::parse(iso_data);
        GameCube::with_ipl_hle(dvd)
    } else if let Some(ref ipl) = args.ipl {
        let ipl_data = std::fs::read(ipl).expect("failed to read IPL");
        GameCube::with_ipl(&ipl_data, args.skip_ipl)
    } else if let Some(ref path) = args.dol {
        let dol_data = std::fs::read(path).expect("failed to read DOL");
        let dol = Dol::parse(dol_data);
        GameCube::with_image(&dol)
    } else {
        eprintln!("error: either --ipl, --ipl-hle, or --dol must be provided");
        std::process::exit(1);
    };

    if !args.ipl_hle {
        if let Some(ref iso) = args.iso {
            if args.ipl.is_none() {
                eprintln!("--iso requires --ipl or --ipl-hle");
                std::process::exit(1);
            }
            let iso_data = std::fs::read(iso).expect("failed to read ISO");
            let dvd = Dvd::parse(iso_data);
            emulator.insert_dvd(dvd);
        }
    }

    if let Some(ref dsp_path) = args.dsp {
        let dsp_data = std::fs::read(dsp_path).expect("failed to read DSP IROM");
        emulator.dsp.load_irom(&dsp_data);
    }

    if let Some(ref path) = args.script {
        let host = scripting::LuaHost::from_file(path).expect("failed to load script");
        emulator.set_hook_host(Box::new(host));
    }

    let symbols = args
        .symbols
        .as_ref()
        .map(|path| image::loader::load_symbols(std::path::Path::new(path)).expect("failed to load symbols"));

    emulator.add_primary_controller(PadStatus {
        connected: true,
        ..PadStatus::default()
    });

    // Create wgpu resources before the event loop (adapter without a surface).
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no compatible wgpu adapter");

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .expect("failed to acquire wgpu device");

    let surface_format = wgpu::TextureFormat::Bgra8Unorm;

    // Create the renderer (spawns the worker thread internally).
    let renderer = backend_wgpu::sink::Renderer::new(device.clone(), queue.clone(), surface_format);

    // Install the renderer as the emulator's render sink.
    emulator.render_sink = Box::new(renderer.clone());

    // Install the EFB-to-texture writeback receiver. Feature-gated.
    #[cfg(feature = "efb-writeback")]
    {
        emulator.gx.efb_writeback_rx = renderer.take_writeback_rx();
    }

    let ui = DebuggerUi {
        symbols,
        ..DebuggerUi::default()
    };

    let event_loop = EventLoop::new().unwrap();
    let mut app = App {
        emulator,
        ui,
        window: None,
        state: None,
        present_mode,
        init: Some(AppInit {
            instance,
            adapter,
            device,
            queue,
            renderer,
            surface_format,
        }),
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
