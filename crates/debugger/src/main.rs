use backend_wgpu::capture::CaptureRequest;
use backend_wgpu::sink::TargetAspect;
use clap::Parser;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use gecko::HostInput;
use gecko::audio::WavAudioSink;
use gecko::flipper::si::pad::{self, PadStatus, STICK_CENTER};
use gecko::gamecube::GameCube;
use gecko::wii::Wii;
use image::Dol;

use crate::debugger::DebuggerUi;
use crate::render::RenderState;

mod debugger;
mod render;

/// Holds either a GameCube or Wii emulator. Runtime dispatch lets the
/// debugger UI work uniformly while keeping `System<SYSTEM>` strongly typed
/// inside each branch.
enum EmulatorVariant {
    Gc(GameCube),
    Wii(Wii),
}

impl EmulatorVariant {
    fn apply_host_input(&mut self, input: &HostInput) {
        match self {
            Self::Gc(e) => e.apply_host_input(input),
            Self::Wii(e) => e.apply_host_input(input),
        }
    }

    fn neutral_input(&self) -> HostInput {
        match self {
            Self::Gc(_) => HostInput::gc_connected(),
            Self::Wii(_) => HostInput::wii_neutral(),
        }
    }

    fn install_render_sink(&mut self, sink: Box<dyn gecko::host::RenderSink>) {
        match self {
            Self::Gc(e) => e.render_sink = sink,
            Self::Wii(e) => e.render_sink = sink,
        }
    }

    #[cfg(feature = "efb-writeback")]
    fn install_efb_writeback(&mut self, rx: Option<crossbeam_channel::Receiver<gecko::host::EfbWriteback>>) {
        match self {
            Self::Gc(e) => e.gx.efb_writeback_rx = rx,
            Self::Wii(e) => e.gx.efb_writeback_rx = rx,
        }
    }

    fn install_recycle_rx(&mut self, rx: Option<crossbeam_channel::Receiver<Box<gecko::host::DrawData>>>) {
        match self {
            Self::Gc(e) => e.gx.draw_box_recycle_rx = rx,
            Self::Wii(e) => e.gx.draw_box_recycle_rx = rx,
        }
    }

    fn load_dsp_irom(&mut self, data: &[u8]) {
        match self {
            Self::Gc(e) => e.dsp.load_irom(data),
            Self::Wii(e) => e.dsp.load_irom(data),
        }
    }

    fn load_dsp_coef(&mut self, data: &[u8]) {
        match self {
            Self::Gc(e) => e.dsp.load_coef(data),
            Self::Wii(e) => e.dsp.load_coef(data),
        }
    }

    fn install_audio_sink(&mut self, sink: Box<dyn gecko::audio::AudioSink>) {
        match self {
            Self::Gc(e) => e.audio_sink = sink,
            Self::Wii(e) => e.audio_sink = sink,
        }
    }

    fn aid_sample_rate_hz(&self) -> u32 {
        match self {
            Self::Gc(e) => e.ai.control.aid_sample_rate_hz(),
            Self::Wii(e) => e.ai.control.aid_sample_rate_hz(),
        }
    }
}

struct App {
    emulator: EmulatorVariant,
    input: HostInput,
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

impl App {
    /// Render one frame against whichever emulator variant is loaded.
    fn render_frame(&mut self) {
        let Some(state) = self.state.as_mut() else { return };
        let Some(window) = self.window.as_ref() else { return };

        // Drain Lua-load request before rendering so the script becomes
        // active starting with this frame's tick.
        if self.ui.lua_load_pending {
            self.ui.lua_load_pending = false;
            match &mut self.emulator {
                EmulatorVariant::Gc(emu) => try_load_lua(emu, &mut self.ui),
                EmulatorVariant::Wii(emu) => try_load_lua(emu, &mut self.ui),
            }
        }

        match &mut self.emulator {
            EmulatorVariant::Gc(emu) => state.render(emu, &mut self.ui, window),
            EmulatorVariant::Wii(emu) => state.render(emu, &mut self.ui, window),
        }
    }
}

fn try_load_lua<const SYSTEM: gecko::SystemId>(emu: &mut gecko::System<SYSTEM>, ui: &mut DebuggerUi) {
    match scripting::LuaHost::from_source("editor", &ui.lua_source) {
        Ok(host) => {
            ui.lua_log.push("[lua] script loaded".to_string());
            emu.set_hook_host(Box::new(host));
        }
        Err(err) => {
            ui.lua_log.push(format!("[lua] error: {err}"));
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let initial_size = self
            .init
            .as_ref()
            .map(|i| initial_window_size(i.renderer.target_aspect()))
            .unwrap_or((1280, 720));
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Gecko Debugger")
                        .with_inner_size(winit::dpi::PhysicalSize::new(initial_size.0, initial_size.1)),
                )
                .unwrap(),
        );

        let Some(init) = self.init.take() else {
            self.window = Some(window);
            return;
        };

        let surface = init.instance.create_surface(window.clone()).unwrap();
        let actual = window.inner_size();
        let (sw, sh) =
            backend_wgpu::sink::snap_size_to_aspect((actual.width, actual.height), init.renderer.target_aspect());
        if (sw, sh) != (actual.width, actual.height) {
            let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(sw, sh));
        }
        let size = winit::dpi::PhysicalSize::new(sw, sh);
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
                if let (Some(state), Some(window)) = (&mut self.state, &self.window) {
                    state.resize(window, size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state.is_pressed();
                if let PhysicalKey::Code(key) = event.physical_key {
                    if pressed && !event.repeat {
                        if let Some(state) = &mut self.state {
                            match key {
                                #[cfg(feature = "renderdoc-capture")]
                                KeyCode::F10 => state.request_renderdoc_capture(),
                                KeyCode::F11 => state.request_screenshot(CaptureRequest::FullWindow),
                                KeyCode::F12 => state.request_screenshot(CaptureRequest::GameOnly),
                                _ => {}
                            }
                        }
                    }

                    if !egui_consumed {
                        if let HostInput::Gc(pad) = &mut self.input {
                            update_pad(pad, key, pressed);
                        }
                        self.emulator.apply_host_input(&self.input);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.render_frame();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

#[derive(Parser)]
#[command(
    about = "GameCube/Wii debugger",
    after_help = "Repository: https://github.com/ioncodes/gecko"
)]
struct Args {
    /// Path to the DOL file (GameCube homebrew by default)
    #[arg(long)]
    dol: Option<String>,

    /// Boot the DOL as a Wii executable instead of GameCube
    #[arg(long)]
    wii: bool,

    /// Path to an IPL file (boot the real GameCube IPL)
    #[arg(long)]
    ipl: Option<String>,

    /// Patch the IPL to skip directly to disc boot
    #[arg(long)]
    skip_ipl: bool,

    /// Path to a disc image (.iso or .rvz). System (GameCube vs Wii) is
    /// autodetected from the disc magic; Wii discs require .rvz format.
    #[arg(long)]
    dvd: Option<String>,

    /// Use immediate present mode (no vsync)
    #[arg(long)]
    immediate: bool,

    /// Path to a DSP IROM binary
    #[arg(long)]
    dsp: Option<String>,

    /// Path to a DSP coefficient ROM binary
    #[arg(long)]
    coef: Option<String>,

    /// Path to a symbol file (ELF, IDA .idb, or .i64)
    #[arg(long)]
    symbols: Option<String>,

    /// Path to a Lua script for scripting hooks
    #[arg(long)]
    script: Option<String>,

    /// Display aspect ratio: auto (16:9 Wii / 4:3 GC), 4:3, 16:9, stretch
    #[arg(long, default_value = "auto")]
    aspect: String,
}

fn initial_window_size(target_aspect: TargetAspect) -> (u32, u32) {
    match target_aspect {
        TargetAspect::Stretch => (1280, 720),
        TargetAspect::Ratio(ar) => {
            let h: u32 = 720;
            let w = ((h as f32) * ar).round() as u32;
            (w, h)
        }
    }
}

fn resolve_aspect(arg: &str, is_wii: bool) -> TargetAspect {
    match arg {
        "auto" => {
            if is_wii {
                TargetAspect::Ratio(16.0 / 9.0)
            } else {
                TargetAspect::Ratio(4.0 / 3.0)
            }
        }
        "4:3" => TargetAspect::Ratio(4.0 / 3.0),
        "16:9" => TargetAspect::Ratio(16.0 / 9.0),
        "stretch" => TargetAspect::Stretch,
        other => panic!("--aspect must be auto|4:3|16:9|stretch, got {other:?}"),
    }
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

    let mut emulator = if let Some(ref dol) = args.dol {
        let dol = Dol::parse(std::fs::read(dol).expect("failed to read DOL"));
        if args.wii {
            EmulatorVariant::Wii(Wii::with_image(&dol))
        } else {
            EmulatorVariant::Gc(GameCube::with_image(&dol))
        }
    } else if let Some(ref ipl) = args.ipl {
        let mut gc = GameCube::with_ipl(&std::fs::read(ipl).expect("failed to read IPL"), args.skip_ipl);
        if let Some(ref dvd) = args.dvd {
            gc.insert_dvd(image::load_dvd(std::fs::read(dvd).expect("failed to read DVD")));
        }
        EmulatorVariant::Gc(gc)
    } else if let Some(ref dvd_path) = args.dvd {
        let dvd_data = std::fs::read(dvd_path).expect("failed to read DVD");
        let dvd = image::load_dvd(dvd_data);
        if dvd.header().is_wii() {
            eprintln!("Detected Wii disc, booting via apploader HLE");
            EmulatorVariant::Wii(Wii::apploader_hle(dvd).build())
        } else {
            eprintln!("Detected GameCube disc, booting via IPL HLE");
            EmulatorVariant::Gc(GameCube::with_ipl_hle(dvd))
        }
    } else {
        panic!("provide one of --dol, --ipl, or --dvd");
    };

    if let Some(ref dsp_path) = args.dsp {
        let dsp_data = std::fs::read(dsp_path).expect("failed to read DSP IROM");
        emulator.load_dsp_irom(&dsp_data);
    }

    if let Some(ref coef_path) = args.coef {
        let coef_data = std::fs::read(coef_path).expect("failed to read DSP coefficient ROM");
        emulator.load_dsp_coef(&coef_data);
    }

    if let Some(ref path) = args.script {
        let host = scripting::LuaHost::from_file(path).expect("failed to load script");
        match &mut emulator {
            EmulatorVariant::Gc(emu) => emu.set_hook_host(Box::new(host)),
            EmulatorVariant::Wii(emu) => emu.set_hook_host(Box::new(host)),
        }
    }

    let symbols = args
        .symbols
        .as_ref()
        .map(|path| image::loader::load_symbols(std::path::Path::new(path)).expect("failed to load symbols"));

    let input = emulator.neutral_input();
    emulator.apply_host_input(&input);

    // Debugger always dumps to WAV file
    let emulated_rate = emulator.aid_sample_rate_hz();
    emulator.install_audio_sink(Box::new(WavAudioSink::create("dump.wav", emulated_rate)));

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

    let target_aspect = resolve_aspect(&args.aspect, matches!(emulator, EmulatorVariant::Wii(_)));
    let renderer = backend_wgpu::sink::Renderer::new(device.clone(), queue.clone(), surface_format, target_aspect);

    emulator.install_recycle_rx(renderer.take_recycle_rx());
    emulator.install_render_sink(Box::new(renderer.take_batching_sink()));

    #[cfg(feature = "efb-writeback")]
    {
        emulator.install_efb_writeback(renderer.take_writeback_rx());
    }

    let ui = DebuggerUi {
        symbols,
        ..DebuggerUi::default()
    };

    let event_loop = EventLoop::new().unwrap();
    let mut app = App {
        emulator,
        input,
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
