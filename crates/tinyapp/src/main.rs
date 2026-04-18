mod app;
mod thread;

use clap::Parser;
use crossbeam_channel::bounded;
use gecko::flipper::si::pad::{self, PadStatus, STICK_CENTER};
use gecko::gamecube::GameCube;
use image::Dol;
use image::dvd::Dvd;
use std::sync::{Arc, Mutex};
use winit::event_loop::EventLoop;
use winit::keyboard::KeyCode;

#[derive(Parser)]
#[command(about = "GameCube emulator")]
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

    /// Disable ANSI escape codes
    #[arg(long)]
    no_ansi: bool,

    /// Path to a DSP IROM binary
    #[arg(long)]
    dsp: Option<String>,

    /// Path to a Lua script for scripting hooks
    #[cfg(feature = "scripting")]
    #[arg(long)]
    script: Option<String>,
}

fn main() {
    let args = Args::parse();

    let present_mode = if args.immediate {
        wgpu::PresentMode::Immediate
    } else {
        wgpu::PresentMode::Fifo
    };

    tracing_subscriber::fmt()
        .without_time()
        .with_ansi(!args.no_ansi)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

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

    #[cfg(feature = "scripting")]
    if let Some(ref path) = args.script {
        let host = scripting::LuaHost::from_file(path).expect("failed to load script");
        emulator.set_hook_host(Box::new(host));
    }

    // Channel 0 always has a controller connected
    emulator.add_primary_controller(PadStatus {
        connected: true,
        ..PadStatus::default()
    });

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

    // Install the EFB-to-texture writeback receiver so the emu thread
    // can copy encoded texture bytes back into RAM after each readback.
    // Only present with the `efb-writeback` feature.
    #[cfg(feature = "efb-writeback")]
    {
        emulator.gx.efb_writeback_rx = renderer.take_writeback_rx();
    }

    let input = Arc::new(Mutex::new(*emulator.primary_controller_mut()));

    let (frame_tx, frame_rx) = bounded::<thread::FrameMessage>(2);

    let event_loop = EventLoop::new().unwrap();
    let proxy = event_loop.create_proxy();

    let emu_input = input.clone();
    std::thread::Builder::new()
        .name("emu".into())
        .spawn(move || thread::emu_thread(emulator, frame_tx, emu_input, proxy))
        .expect("failed to spawn emulator thread");

    let mut app = app::App {
        frame_rx,
        input,
        window: None,
        state: None,
        present_mode,
        init: Some(app::AppInit {
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
        // Analog stick
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
