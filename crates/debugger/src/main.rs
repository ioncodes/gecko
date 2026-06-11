use backend_wgpu::capture::CaptureRequest;
use backend_wgpu::sink::TargetAspect;
use clap::Parser;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use gecko::HostInput;
use gecko::audio::{EmptyAudioSink, WavAudioSink};
use gecko::flipper::si::pad::{self, PadStatus, STICK_CENTER, STICK_MAX, STICK_MIN, TRIGGER_MAX, TRIGGER_MIN};
use gecko::gamecube::GameCube;
use gecko::hollywood::ipc::usb as wiimote;
use gecko::wii::Wii;
use image::Dol;

#[derive(Debug, Clone, Copy)]
pub enum UserEvent {
    Shutdown,
}

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

    fn set_execution_mode(&mut self, mode: gecko::ExecutionMode) {
        match self {
            Self::Gc(e) => e.set_execution_mode(mode),
            Self::Wii(e) => e.set_execution_mode(mode),
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

    fn load_jit_cache(&mut self, game_id: &str) -> (usize, usize, usize, usize, usize, usize) {
        match self {
            Self::Gc(e) => e.load_jit_cache(game_id),
            Self::Wii(e) => e.load_jit_cache(game_id),
        }
    }

    fn save_jit_cache(&self, game_id: &str) -> std::io::Result<(usize, usize, usize)> {
        match self {
            Self::Gc(e) => e.save_jit_cache(game_id),
            Self::Wii(e) => e.save_jit_cache(game_id),
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

impl ApplicationHandler<UserEvent> for App {
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Shutdown => event_loop.exit(),
        }
    }

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
                        match &mut self.input {
                            HostInput::Gc(pad) => update_pad(pad, key, pressed),
                            HostInput::Wii {
                                wiimote_buttons,
                                wiimote_shake,
                                nunchuk_buttons,
                                nunchuk_stick_x,
                                nunchuk_stick_y,
                                ..
                            } => {
                                update_wiimote_keys(wiimote_buttons, key, pressed);
                                update_wiimote_motion_keys(wiimote_shake, key, pressed);
                                update_nunchuk_keys(nunchuk_buttons, nunchuk_stick_x, nunchuk_stick_y, key, pressed);
                            }
                        }
                        self.emulator.apply_host_input(&self.input);
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state.is_pressed();
                let mask = match button {
                    MouseButton::Left => Some(wiimote::BTN_A),
                    MouseButton::Right => Some(wiimote::BTN_B),
                    _ => None,
                };

                if let Some(mask) = mask
                    && let HostInput::Wii { wiimote_buttons, .. } = &mut self.input
                {
                    set_bit(wiimote_buttons, mask, pressed);
                    self.emulator.apply_host_input(&self.input);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let Some(window) = &self.window else {
                    return;
                };

                let size = window.inner_size();
                if size.width == 0 || size.height == 0 {
                    return;
                }

                const POINTER_SCALE_X: f64 = 0.44;
                const POINTER_SCALE_Y: f64 = 0.66;
                const POINTER_Y_OFFSET: f64 = 120.0;

                let aim_x = (position.x / size.width as f64).clamp(0.0, 1.0);
                let aim_y = (position.y / size.height as f64).clamp(0.0, 1.0);
                let span_x = wiimote::IR_CAMERA_WIDTH as f64 * POINTER_SCALE_X;
                let span_y = wiimote::IR_CAMERA_HEIGHT as f64 * POINTER_SCALE_Y;
                let base_x = (wiimote::IR_CAMERA_WIDTH as f64 - span_x) / 2.0;
                let base_y = (wiimote::IR_CAMERA_HEIGHT as f64 - span_y) / 2.0 + POINTER_Y_OFFSET;
                let ir_x = (base_x + (1.0 - aim_x) * span_x) as u16;
                let ir_y = (base_y + aim_y * span_y) as u16;

                if let HostInput::Wii { ir_pointer, .. } = &mut self.input
                    && *ir_pointer != Some((ir_x, ir_y))
                {
                    *ir_pointer = Some((ir_x, ir_y));
                    self.emulator.apply_host_input(&self.input);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if let HostInput::Wii { ir_pointer, .. } = &mut self.input {
                    *ir_pointer = None;
                    self.emulator.apply_host_input(&self.input);
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

    /// Force interpreter dispatch for CPU/DSP/Vertex (default: JIT)
    #[arg(long)]
    interpreter: bool,
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

fn main() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        .add_directive("cranelift_jit=warn".parse().unwrap())
        .add_directive("cranelift_codegen=warn".parse().unwrap())
        .add_directive("cranelift_frontend=warn".parse().unwrap())
        .add_directive("regalloc2=warn".parse().unwrap())
        .add_directive("wgpu_core=warn".parse().unwrap())
        .add_directive("wgpu_hal=warn".parse().unwrap())
        .add_directive("naga=warn".parse().unwrap());

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .without_time()
        .init();

    let args = Args::parse();

    let present_mode = if args.immediate {
        wgpu::PresentMode::Immediate
    } else {
        wgpu::PresentMode::Fifo
    };

    let mut game_id: Option<String> = None;
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
        game_id = Some(dvd.header().game_id());
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

    emulator.set_execution_mode(if args.interpreter {
        gecko::ExecutionMode::Interpreter
    } else {
        gecko::ExecutionMode::Jit
    });

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
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
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

    let target_aspect = TargetAspect::from_arg(&args.aspect, matches!(emulator, EmulatorVariant::Wii(_)));
    let (renderer, sink) =
        backend_wgpu::sink::Renderer::new(device.clone(), queue.clone(), surface_format, target_aspect);

    emulator.install_render_sink(Box::new(sink));

    let ui = DebuggerUi {
        symbols,
        ..DebuggerUi::default()
    };

    if let Some(ref id) = game_id {
        let (ppc_c, ppc_s, dsp_c, dsp_s, vtx_c, vtx_s) = emulator.load_jit_cache(id);
        if ppc_c > 0 || dsp_c > 0 || vtx_c > 0 || ppc_s > 0 || dsp_s > 0 || vtx_s > 0 {
            eprintln!("JIT cache loaded for {id}: ppc {ppc_c}/{ppc_s}, dsp {dsp_c}/{dsp_s}, vtx {vtx_c}/{vtx_s}");
        }
    }

    let event_loop = EventLoop::<UserEvent>::with_user_event().build().unwrap();

    {
        let proxy = event_loop.create_proxy();
        let pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        if let Err(err) = ctrlc::set_handler(move || {
            if pending.swap(true, std::sync::atomic::Ordering::Relaxed) {
                std::process::exit(69);
            }
            eprintln!("Ctrl+C received, requesting graceful shutdown");
            let _ = proxy.send_event(UserEvent::Shutdown);
        }) {
            eprintln!("failed to install Ctrl+C handler: {err:?}");
        }
    }

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

    if let Some(ref id) = game_id {
        match app.emulator.save_jit_cache(id) {
            Ok((ppc, dsp, vtx)) => {
                eprintln!("saved JIT cache: ppc {ppc} dsp {dsp} vtx {vtx}");
            }
            Err(err) => eprintln!("failed to save JIT cache: {err}"),
        }
    }

    app.emulator.install_audio_sink(Box::new(EmptyAudioSink));

    std::mem::forget(app);
}

#[inline(always)]
fn set_bit<T: std::ops::BitOrAssign + std::ops::BitAndAssign + std::ops::Not<Output = T> + Copy>(
    bits: &mut T,
    mask: T,
    on: bool,
) {
    if on {
        *bits |= mask;
    } else {
        *bits &= !mask;
    }
}

fn update_pad(pad: &mut PadStatus, key: KeyCode, pressed: bool) {
    match key {
        // Analog stick (digital, full deflection)
        KeyCode::ArrowUp => pad.stick_y = if pressed { STICK_MAX } else { STICK_CENTER },
        KeyCode::ArrowDown => pad.stick_y = if pressed { STICK_MIN } else { STICK_CENTER },
        KeyCode::ArrowLeft => pad.stick_x = if pressed { STICK_MIN } else { STICK_CENTER },
        KeyCode::ArrowRight => pad.stick_x = if pressed { STICK_MAX } else { STICK_CENTER },

        // Face buttons
        KeyCode::KeyX => set_bit(&mut pad.buttons, pad::A, pressed),
        KeyCode::KeyZ => set_bit(&mut pad.buttons, pad::B, pressed),
        KeyCode::KeyC => set_bit(&mut pad.buttons, pad::X, pressed),
        KeyCode::KeyV => set_bit(&mut pad.buttons, pad::Y, pressed),
        KeyCode::Enter => set_bit(&mut pad.buttons, pad::START, pressed),

        // Triggers
        KeyCode::KeyA => {
            set_bit(&mut pad.buttons, pad::L, pressed);
            pad.trigger_left = if pressed { TRIGGER_MAX } else { TRIGGER_MIN };
        }
        KeyCode::KeyS => {
            set_bit(&mut pad.buttons, pad::R, pressed);
            pad.trigger_right = if pressed { TRIGGER_MAX } else { TRIGGER_MIN };
        }
        KeyCode::KeyD => set_bit(&mut pad.buttons, pad::Z, pressed),

        // D-pad
        KeyCode::KeyI => set_bit(&mut pad.buttons, pad::DPAD_UP, pressed),
        KeyCode::KeyK => set_bit(&mut pad.buttons, pad::DPAD_DOWN, pressed),
        KeyCode::KeyJ => set_bit(&mut pad.buttons, pad::DPAD_LEFT, pressed),
        KeyCode::KeyL => set_bit(&mut pad.buttons, pad::DPAD_RIGHT, pressed),
        _ => {}
    }
}

fn update_wiimote_keys(buttons: &mut u16, key: KeyCode, pressed: bool) {
    let mask = match key {
        KeyCode::Digit1 => wiimote::BTN_ONE,
        KeyCode::Digit2 => wiimote::BTN_TWO,
        KeyCode::Home => wiimote::BTN_HOME,
        KeyCode::Minus => wiimote::BTN_MINUS,
        KeyCode::Equal => wiimote::BTN_PLUS,
        KeyCode::ArrowUp => wiimote::BTN_UP,
        KeyCode::ArrowDown => wiimote::BTN_DOWN,
        KeyCode::ArrowLeft => wiimote::BTN_LEFT,
        KeyCode::ArrowRight => wiimote::BTN_RIGHT,
        _ => return,
    };
    set_bit(buttons, mask, pressed);
}

fn update_wiimote_motion_keys(shake: &mut bool, key: KeyCode, pressed: bool) {
    if let KeyCode::ShiftLeft = key {
        *shake = pressed;
    }
}

fn update_nunchuk_keys(buttons: &mut u8, stick_x: &mut u8, stick_y: &mut u8, key: KeyCode, pressed: bool) {
    use wiimote::{NUNCHUK_STICK_CENTER as C, NUNCHUK_STICK_MAX as MAX, NUNCHUK_STICK_MIN as MIN};
    match key {
        KeyCode::KeyW => *stick_y = if pressed { MAX } else { C },
        KeyCode::KeyS => *stick_y = if pressed { MIN } else { C },
        KeyCode::KeyA => *stick_x = if pressed { MIN } else { C },
        KeyCode::KeyD => *stick_x = if pressed { MAX } else { C },
        KeyCode::KeyQ => set_bit(buttons, wiimote::NUNCHUK_BTN_Z, pressed),
        KeyCode::KeyE => set_bit(buttons, wiimote::NUNCHUK_BTN_C, pressed),
        _ => {}
    }
}
