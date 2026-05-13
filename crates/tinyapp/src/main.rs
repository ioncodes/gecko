mod app;
mod audio;
mod thread;

#[cfg(not(feature = "hotpath-alloc"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "hotpath-alloc")]
#[derive(Default)]
struct DefaultMiMalloc;
#[cfg(feature = "hotpath-alloc")]
unsafe impl std::alloc::GlobalAlloc for DefaultMiMalloc {
    #[inline]
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        unsafe { std::alloc::GlobalAlloc::alloc(&mimalloc::MiMalloc, layout) }
    }
    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::GlobalAlloc::dealloc(&mimalloc::MiMalloc, ptr, layout) }
    }
    #[inline]
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        unsafe { std::alloc::GlobalAlloc::alloc_zeroed(&mimalloc::MiMalloc, layout) }
    }
    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        unsafe { std::alloc::GlobalAlloc::realloc(&mimalloc::MiMalloc, ptr, layout, new_size) }
    }
}

#[cfg(feature = "hotpath-alloc")]
#[global_allocator]
static GLOBAL: hotpath::CountingAllocator<DefaultMiMalloc> = hotpath::CountingAllocator::new();

use backend_wgpu::sink::TargetAspect;
use clap::Parser;
use gecko::HostInput;
#[cfg(feature = "audio-wav-dump")]
use gecko::audio::WavAudioSink;
use gecko::audio::{AudioSink, EmptyAudioSink, MultiplexAudioSink};
use gecko::flipper::si::pad::{self, PadStatus, STICK_CENTER, STICK_MAX, STICK_MIN, TRIGGER_MAX, TRIGGER_MIN};
use gecko::gamecube::GameCube;
use gecko::hollywood::ipc::usb as wiimote;
use gecko::system::{self, System, SystemId};
use gecko::wii::Wii;
use image::Dol;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use winit::event_loop::EventLoop;
use winit::keyboard::KeyCode;

#[derive(Debug, Clone, Copy)]
pub enum UserEvent {
    FrameReady { at: Instant },
    Shutdown,
}

#[derive(Parser)]
#[command(
    about = "GameCube/Wii emulator",
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

    /// Disable ANSI escape codes
    #[arg(long)]
    no_ansi: bool,

    /// Path to a DSP IROM binary
    #[arg(long)]
    dsp: Option<String>,

    /// Path to a DSP coefficient ROM binary
    #[arg(long)]
    coef: Option<String>,

    /// Path to a Lua script for scripting hooks
    #[cfg(feature = "scripting")]
    #[arg(long)]
    script: Option<String>,

    /// Display aspect ratio: auto (16:9 Wii / 4:3 GC), 4:3, 16:9, stretch
    #[arg(long, default_value = "auto")]
    aspect: String,

    /// Disable audio output
    #[arg(long)]
    no_audio: bool,

    /// Disable audio-driven framerate cap; emu runs as fast as possible
    #[arg(long)]
    no_cap: bool,

    /// Enable per-block JIT heatmap CSV dumps (requires --features jit-stats)
    #[cfg(feature = "jit-stats")]
    #[arg(long)]
    heatmap: bool,

    /// Frames between heatmap CSV writes
    #[cfg(feature = "jit-stats")]
    #[arg(long, default_value_t = 60)]
    heatmap_interval_frames: u32,

    /// Output directory for heatmap CSV files
    #[cfg(feature = "jit-stats")]
    #[arg(long, default_value = "./profile-dumps")]
    heatmap_out: String,

    /// Top-K rows per heatmap CSV
    #[cfg(feature = "jit-stats")]
    #[arg(long, default_value_t = 64)]
    heatmap_top_k: usize,

    /// Run an in-process pprof sampling profile for N seconds, then write a
    /// flamegraph SVG (requires --features profile)
    #[cfg(feature = "profile")]
    #[arg(long)]
    pprof_secs: Option<u32>,

    /// Sampling rate (Hz) for pprof
    #[cfg(feature = "profile")]
    #[arg(long, default_value_t = 997)]
    pprof_hz: u32,

    /// Output path for the pprof samples CSV (rank,samples,pct,symbol)
    #[cfg(feature = "profile")]
    #[arg(long, default_value = "./profile-dumps/pprof-samples.csv")]
    pprof_out: String,

    /// Delay starting pprof for N emulated vsyncs (e.g. 3600 = 60s of in-game vsyncs)
    #[cfg(feature = "profile")]
    #[arg(long, default_value_t = 0)]
    pprof_delay: u32,

    /// Append per-second emu FPS samples to a CSV at this path (requires --features fps-counter)
    #[cfg(feature = "fps-counter")]
    #[arg(long)]
    fps_log: Option<String>,

    /// Show a small centered window and block until space is pressed before booting
    #[arg(long)]
    wait: bool,
}

fn resolve_aspect(arg: &str, system: SystemId) -> TargetAspect {
    match arg {
        "auto" => {
            if system == system::WII {
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
    #[cfg(feature = "hotpath")]
    let _hotpath_guard = hotpath::HotpathGuardBuilder::new("main")
        .percentiles(&[50.0, 95.0, 99.0])
        .functions_limit(64)
        .build();

    let args = Args::parse();

    let present_mode = if args.immediate {
        wgpu::PresentMode::Immediate
    } else {
        wgpu::PresentMode::Fifo
    };

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"))
        .add_directive("cranelift_jit=warn".parse().unwrap())
        .add_directive("cranelift_codegen=warn".parse().unwrap())
        .add_directive("cranelift_frontend=warn".parse().unwrap())
        .add_directive("regalloc2=warn".parse().unwrap())
        .add_directive("wgpu_core=warn".parse().unwrap())
        .add_directive("wgpu_hal=warn".parse().unwrap())
        .add_directive("naga=warn".parse().unwrap());

    tracing_subscriber::fmt()
        .without_time()
        .with_ansi(!args.no_ansi)
        .with_env_filter(env_filter)
        .init();

    // Boot dispatch:
    //   --dol           : GameCube homebrew (with_image), or Wii if --wii
    //   --ipl [+ --dvd] : GameCube real IPL boot (with_ipl)
    //   --dvd <path>    : autodetect Wii vs GC, HLE boot via apploader
    if let Some(ref dol) = args.dol {
        let dol = Dol::parse(std::fs::read(dol).expect("failed to read DOL"));
        if args.wii {
            let mut emulator = Wii::with_image(&dol);
            configure(&mut emulator, &args);
            run(emulator, present_mode, &args, None);
        } else {
            let mut emulator = GameCube::with_image(&dol);
            configure(&mut emulator, &args);
            run(emulator, present_mode, &args, None);
        }
    } else if let Some(ref ipl_path) = args.ipl {
        let ipl_data = std::fs::read(ipl_path).expect("failed to read IPL");
        let mut emulator = GameCube::with_ipl(&ipl_data, args.skip_ipl);
        if let Some(ref dvd_path) = args.dvd {
            let dvd_data = std::fs::read(dvd_path).expect("failed to read DVD");
            emulator.insert_dvd(image::load_dvd(dvd_data));
        }
        configure(&mut emulator, &args);
        run(emulator, present_mode, &args, None);
    } else if let Some(ref dvd_path) = args.dvd {
        let dvd_data = std::fs::read(dvd_path).expect("failed to read DVD");
        let dvd = image::load_dvd(dvd_data);
        let game_id = self::game_id_from_header(dvd.header());
        if dvd.header().is_wii() {
            tracing::info!("Detected Wii disc, booting via apploader HLE");
            let builder = Wii::apploader_hle(dvd);
            #[cfg(feature = "scripting")]
            let builder = if let Some(ref path) = args.script {
                let host = scripting::LuaHost::from_file(path).expect("failed to load script");
                builder.lua_host(Box::new(host))
            } else {
                builder
            };
            let mut emulator = builder.build();
            configure(&mut emulator, &args);
            run(emulator, present_mode, &args, Some(game_id));
        } else {
            tracing::info!("Detected GameCube disc, booting via IPL HLE");
            let mut emulator = GameCube::with_ipl_hle(dvd);
            configure(&mut emulator, &args);
            run(emulator, present_mode, &args, Some(game_id));
        }
    } else {
        panic!("provide one of --dol, --ipl, or --dvd");
    }
}

fn game_id_from_header(header: &image::dvd::Header) -> String {
    let mut buf = String::with_capacity(6);

    for &b in &header.game_code {
        buf.push(if b.is_ascii_graphic() { b as char } else { '_' });
    }

    for &b in &header.maker_code {
        buf.push(if b.is_ascii_graphic() { b as char } else { '_' });
    }

    buf
}

fn configure<const SYSTEM: SystemId>(emulator: &mut System<SYSTEM>, args: &Args) {
    if let Some(ref dsp_path) = args.dsp {
        let dsp_data = std::fs::read(dsp_path).expect("failed to read DSP IROM");
        emulator.dsp.load_irom(&dsp_data);
    }

    if let Some(ref coef_path) = args.coef {
        let coef_data = std::fs::read(coef_path).expect("failed to read DSP coefficient ROM");
        emulator.dsp.load_coef(&coef_data);
    }

    #[cfg(feature = "scripting")]
    if let Some(ref path) = args.script {
        // Already attached for Wii apploader HLE; only attach here for other
        // boot paths.
        if !emulator.has_hook_host() {
            let host = scripting::LuaHost::from_file(path).expect("failed to load script");
            emulator.set_hook_host(Box::new(host));
        }
    }

    emulator.apply_host_input(&HostInput::neutral_for(SYSTEM));

    #[cfg(feature = "jit-stats")]
    {
        emulator.heatmap = gecko::profile::HeatmapConfig {
            enabled: args.heatmap,
            interval_frames: args.heatmap_interval_frames,
            out_dir: std::path::PathBuf::from(&args.heatmap_out),
            top_k: args.heatmap_top_k,
        };
    }

    #[cfg(feature = "profile")]
    if let Some(secs) = args.pprof_secs {
        emulator.pprof_config = Some(gecko::profile::PprofConfig {
            hz: args.pprof_hz,
            secs,
            out: std::path::PathBuf::from(&args.pprof_out),
            delay_vsyncs: args.pprof_delay,
        });
    }
}

fn run<const SYSTEM: SystemId>(
    mut emulator: System<SYSTEM>,
    present_mode: wgpu::PresentMode,
    args: &Args,
    game_id: Option<String>,
) {
    let target_aspect = resolve_aspect(&args.aspect, SYSTEM);
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

    let (renderer, renderer_worker) =
        backend_wgpu::sink::Renderer::new(device.clone(), queue.clone(), surface_format, target_aspect);

    emulator.gx.draw_box_recycle_rx = renderer.take_recycle_rx();
    emulator.render_sink = Box::new(renderer.take_batching_sink());

    #[cfg(feature = "efb-writeback")]
    {
        emulator.gx.efb_writeback_rx = renderer.take_writeback_rx();
    }

    let audio_stream = install_audio_sink(args, &mut emulator);

    #[cfg(feature = "fps-counter")]
    let fps_shared = emulator.fps_counter.shared();
    #[cfg(feature = "fps-counter")]
    if let Some(ref path) = args.fps_log {
        emulator.fps_counter.log_path = Some(std::path::PathBuf::from(path));
        let _ = std::fs::write(path, "wall_secs,fps,native_pct,vsyncs\n");
    }

    let input = Arc::new(Mutex::new(HostInput::neutral_for(SYSTEM)));

    let event_loop = EventLoop::<UserEvent>::with_user_event().build().unwrap();
    let proxy = event_loop.create_proxy();

    {
        let cb_proxy = proxy.clone();
        renderer.set_frame_ready_callback(move |at| {
            let _ = cb_proxy.send_event(UserEvent::FrameReady { at });
        });
    }

    // Ctrl+C / SIGINT routes through the winit event loop so it tears down
    // through the same path as the window close button. Crucial for the
    // WAV dump: hound only patches the RIFF header sizes when the writer
    // is dropped and Drop only runs if the emu thread is joined cleanly.
    let shutdown_requested = Arc::new(AtomicBool::new(false));
    {
        let shutdown = shutdown_requested.clone();
        let proxy = event_loop.create_proxy();
        if let Err(err) = ctrlc::set_handler(move || {
            if shutdown.swap(true, Ordering::Relaxed) {
                // Motherfucker wants to hard exit so let him
                std::process::exit(69);
            }

            tracing::info!("Ctrl+C received, requesting graceful shutdown");
            let _ = proxy.send_event(UserEvent::Shutdown);
        }) {
            tracing::warn!(?err, "failed to install Ctrl+C handler");
        }
    }

    let emu_input = input.clone();
    if let Some(ref id) = game_id {
        let (ppc_c, ppc_s, dsp_c, dsp_s, vtx_c, vtx_s) = emulator.load_jit_cache(id);
        if ppc_c > 0 || dsp_c > 0 || vtx_c > 0 || ppc_s > 0 || dsp_s > 0 || vtx_s > 0 {
            tracing::info!(
                game = id.as_str(),
                ppc_compiled = ppc_c,
                ppc_skipped = ppc_s,
                dsp_compiled = dsp_c,
                dsp_skipped = dsp_s,
                vtx_compiled = vtx_c,
                vtx_skipped = vtx_s,
                "JIT cache loaded",
            );
        }
    }

    let throttle = !args.no_cap;
    let start_gate = Arc::new(AtomicBool::new(!args.wait));
    let emu_start_gate = start_gate.clone();
    let emu_shutdown = shutdown_requested.clone();
    drop(proxy);
    let emu_handle = std::thread::Builder::new()
        .name("emu".into())
        .spawn(move || {
            thread::emu_thread::<SYSTEM>(emulator, emu_input, game_id, throttle, emu_start_gate, emu_shutdown)
        })
        .expect("failed to spawn emulator thread");

    let mut app = app::App {
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
        _audio_stream: audio_stream,
        shutdown_requested,
        start_gate,
        #[cfg(feature = "fps-counter")]
        fps_shared,
    };
    event_loop.run_app(&mut app).unwrap();

    if let Err(err) = emu_handle.join() {
        tracing::error!(?err, "emu thread panicked");
    }

    if let Err(err) = renderer_worker.shutdown_and_join() {
        tracing::error!(?err, "gx-renderer thread panicked");
    }

    std::mem::forget(app);
}

fn install_audio_sink<const SYSTEM: SystemId>(args: &Args, emulator: &mut System<SYSTEM>) -> Option<cpal::Stream> {
    let emulated_rate = emulator.ai.control.aid_sample_rate_hz();

    let mut sinks: Vec<Box<dyn AudioSink>> = Vec::new();
    let mut stream: Option<cpal::Stream> = None;

    if !args.no_audio {
        match audio::open(emulated_rate) {
            Ok(backend) => {
                sinks.push(Box::new(backend.sink));
                stream = Some(backend.stream);
            }
            Err(err) => {
                tracing::warn!(?err, "Failed to open CPAL output; running silent");
            }
        }
    }

    #[cfg(feature = "audio-wav-dump")]
    sinks.push(Box::new(WavAudioSink::create("dump.wav", emulated_rate)));

    emulator.audio_sink = match sinks.len() {
        0 => Box::new(EmptyAudioSink),
        1 => sinks.into_iter().next().unwrap(),
        _ => Box::new(MultiplexAudioSink::new(sinks)),
    };

    stream
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
        // Analog stick
        KeyCode::ArrowUp => pad.stick_y = if pressed { STICK_MAX } else { STICK_CENTER },
        KeyCode::ArrowDown => pad.stick_y = if pressed { STICK_MIN } else { STICK_CENTER },
        KeyCode::ArrowLeft => pad.stick_x = if pressed { STICK_MIN } else { STICK_CENTER },
        KeyCode::ArrowRight => pad.stick_x = if pressed { STICK_MAX } else { STICK_CENTER },

        // Face buttons
        KeyCode::KeyX => self::set_bit(&mut pad.buttons, pad::A, pressed),
        KeyCode::KeyZ => self::set_bit(&mut pad.buttons, pad::B, pressed),
        KeyCode::KeyC => self::set_bit(&mut pad.buttons, pad::X, pressed),
        KeyCode::KeyV => self::set_bit(&mut pad.buttons, pad::Y, pressed),
        KeyCode::Enter => self::set_bit(&mut pad.buttons, pad::START, pressed),

        // Triggers
        KeyCode::KeyA => {
            self::set_bit(&mut pad.buttons, pad::L, pressed);
            pad.trigger_left = if pressed { TRIGGER_MAX } else { TRIGGER_MIN };
        }
        KeyCode::KeyS => {
            self::set_bit(&mut pad.buttons, pad::R, pressed);
            pad.trigger_right = if pressed { TRIGGER_MAX } else { TRIGGER_MIN };
        }
        KeyCode::KeyD => self::set_bit(&mut pad.buttons, pad::Z, pressed),

        // D-pad
        KeyCode::KeyI => self::set_bit(&mut pad.buttons, pad::DPAD_UP, pressed),
        KeyCode::KeyK => self::set_bit(&mut pad.buttons, pad::DPAD_DOWN, pressed),
        KeyCode::KeyJ => self::set_bit(&mut pad.buttons, pad::DPAD_LEFT, pressed),
        KeyCode::KeyL => self::set_bit(&mut pad.buttons, pad::DPAD_RIGHT, pressed),

        _ => {}
    }
}

pub fn update_wiimote_keys(buttons: &mut u16, key: KeyCode, pressed: bool) {
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
    self::set_bit(buttons, mask, pressed);
}

pub fn update_wiimote_motion_keys(shake: &mut bool, key: KeyCode, pressed: bool) {
    if let KeyCode::ShiftLeft = key {
        *shake = pressed;
    }
}

pub fn update_nunchuk_keys(buttons: &mut u8, stick_x: &mut u8, stick_y: &mut u8, key: KeyCode, pressed: bool) {
    use wiimote::{NUNCHUK_STICK_CENTER as C, NUNCHUK_STICK_MAX as MAX, NUNCHUK_STICK_MIN as MIN};
    match key {
        KeyCode::KeyW => *stick_y = if pressed { MAX } else { C },
        KeyCode::KeyS => *stick_y = if pressed { MIN } else { C },
        KeyCode::KeyA => *stick_x = if pressed { MIN } else { C },
        KeyCode::KeyD => *stick_x = if pressed { MAX } else { C },
        KeyCode::KeyQ => self::set_bit(buttons, wiimote::NUNCHUK_BTN_Z, pressed),
        KeyCode::KeyE => self::set_bit(buttons, wiimote::NUNCHUK_BTN_C, pressed),
        _ => {}
    }
}
