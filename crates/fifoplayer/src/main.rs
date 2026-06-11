mod debug;
mod playback;

use backend_wgpu::sink::{InlineSink, TargetAspect};
use clap::Parser;
use gecko::system::{GC, WII};
use std::path::PathBuf;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::playback::{Playback, PlayerSink};

#[derive(Parser)]
#[command(about = "FIFO player (compatible with Dolphin)")]
struct Args {
    /// Path to the .dff FIFO log
    file: PathBuf,

    /// First frame of the playback range
    #[arg(long, default_value_t = 0)]
    start: usize,

    /// Last frame of the playback range (inclusive, default: last)
    #[arg(long)]
    end: Option<usize>,

    /// Play the range once instead of looping (windowed mode)
    #[arg(long)]
    once: bool,

    /// Headless: play up to --end once, write the presented XFB as PNG, exit
    #[arg(long)]
    screenshot: Option<PathBuf>,

    /// Headless: dump the decoded texture cache as PNGs into this directory
    /// after playback
    #[arg(long)]
    dump_textures: Option<PathBuf>,

    /// Display aspect ratio: auto (16:9 Wii / 4:3 GC), 4:3, 16:9, stretch
    #[arg(long, default_value = "auto")]
    aspect: String,

    /// Open the FIFO debugger (stepping, disassembly, editing). With
    /// --screenshot, replays through the stepping engine headlessly instead.
    #[arg(long)]
    debug: bool,
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

    let mut file = dff::DffFile::load(&args.file).unwrap_or_else(|e| {
        eprintln!("failed to load {}: {e}", args.file.display());
        std::process::exit(1);
    });

    if file.frames.is_empty() {
        eprintln!("{}: no frames", args.file.display());
        std::process::exit(1);
    }

    for frame in &mut file.frames {
        frame.memory_updates.sort_by_key(|u| u.fifo_position);
    }

    eprintln!(
        "{}: {} [{}] {} frames, version {}",
        args.file.display(),
        file.game_id,
        if file.is_wii { "Wii" } else { "GC" },
        file.frames.len(),
        file.version,
    );

    if file.is_wii {
        self::run::<WII>(file, args);
    } else {
        self::run::<GC>(file, args);
    }
}

fn run<const SYSTEM: gecko::SystemId>(file: dff::DffFile, args: Args) {
    let last = file.frames.len() - 1;
    let start = args.start.min(last);
    let end = args.end.unwrap_or(last).clamp(start, last);

    if args.debug {
        if let Some(ref out) = args.screenshot {
            debug::ui::run_headless::<SYSTEM>(file, args.file, start, end, out);
        } else {
            debug::ui::run_debug::<SYSTEM>(file, args.file, start, end);
        }
    } else if let Some(ref out) = args.screenshot {
        self::run_headless::<SYSTEM>(&file, start, end, out, args.dump_textures.as_deref());
    } else {
        self::run_windowed::<SYSTEM>(file, start, end, args.once, &args.aspect);
    }
}

pub(crate) fn init_wgpu() -> (wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue) {
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

    (instance, adapter, device, queue)
}

fn run_headless<const SYSTEM: gecko::SystemId>(
    file: &dff::DffFile,
    start: usize,
    end: usize,
    out: &PathBuf,
    dump_textures: Option<&std::path::Path>,
) {
    let (_instance, _adapter, device, queue) = self::init_wgpu();

    let (gx, sink) = InlineSink::new(device.clone(), queue.clone(), wgpu::TextureFormat::Rgba8Unorm);
    let mut sink = PlayerSink::new(Box::new(sink));

    let mut playback = Playback::<SYSTEM>::new();
    playback.load_state(file, &mut sink);

    let mut presented = 0usize;

    for frame in &file.frames[start..=end] {
        if playback.play_frame(frame, &mut sink) {
            presented += 1;
        }
    }

    eprintln!("played frames {start}..={end}, {presented} presents");

    if let Some(dir) = dump_textures {
        use gecko::host::RenderSink;
        sink.exec(gecko::host::GxAction::DumpTextures { dir: dir.to_path_buf() });
    }

    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });

    let g = gx.lock().unwrap();
    let captured = backend_wgpu::capture::capture_texture(&device, &queue, &g.xfb_texture).expect("XFB capture failed");
    backend_wgpu::capture::write_png(out, captured, true).expect("failed to write PNG");
    eprintln!("wrote {}", out.display());
}

struct WindowedApp<const SYSTEM: gecko::SystemId> {
    file: dff::DffFile,
    playback: Playback<SYSTEM>,
    sink: PlayerSink,
    start: usize,
    end: usize,
    once: bool,
    frame_idx: usize,
    finished: bool,
    window: Option<Arc<Window>>,
    surface: Option<(wgpu::Surface<'static>, wgpu::SurfaceConfiguration)>,
    renderer: backend_wgpu::sink::Renderer,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_format: wgpu::TextureFormat,
}

impl<const SYSTEM: gecko::SystemId> WindowedApp<SYSTEM> {
    fn step(&mut self) {
        if self.finished {
            return;
        }

        self.playback
            .play_frame(&self.file.frames[self.frame_idx], &mut self.sink);

        if self.frame_idx >= self.end {
            if self.once {
                self.finished = true;
            } else {
                self.frame_idx = self.start;
                self.playback.load_state(&self.file, &mut self.sink);
            }
        } else {
            self.frame_idx += 1;
        }
    }
}

impl<const SYSTEM: gecko::SystemId> ApplicationHandler for WindowedApp<SYSTEM> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Gecko — FIFO Player")
                        .with_inner_size(winit::dpi::PhysicalSize::new(1280, 960)),
                )
                .unwrap(),
        );

        let surface = self.instance.create_surface(window.clone()).unwrap();
        let actual = window.inner_size();

        let (sw, sh) =
            backend_wgpu::sink::snap_size_to_aspect((actual.width, actual.height), self.renderer.target_aspect());
        if (sw, sh) != (actual.width, actual.height) {
            let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(sw, sh));
        }

        let surface_caps = surface.get_capabilities(&self.adapter);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: self.surface_format,
            width: sw.max(1),
            height: sh.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&self.device, &surface_config);

        window.request_redraw();

        self.window = Some(window);
        self.surface = Some((surface, surface_config));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some((surface, config)), Some(window)) = (&mut self.surface, &self.window) {
                    if size.width == 0 || size.height == 0 {
                        return;
                    }

                    let (sw, sh) = backend_wgpu::sink::snap_size_to_aspect(
                        (size.width, size.height),
                        self.renderer.target_aspect(),
                    );
                    if (sw, sh) != (size.width, size.height) {
                        let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(sw, sh));
                    }

                    config.width = sw;
                    config.height = sh;
                    surface.configure(&self.device, config);
                }
            }
            WindowEvent::RedrawRequested => {
                self.step();

                let Some((surface, config)) = &self.surface else { return };
                let frame = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                    status => {
                        eprintln!("surface error: {status:?}");
                        return;
                    }
                };
                let view = frame.texture.create_view(&Default::default());

                self.renderer.blit(&self.queue, &view, (config.width, config.height));
                frame.present();

                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn run_windowed<const SYSTEM: gecko::SystemId>(file: dff::DffFile, start: usize, end: usize, once: bool, aspect: &str) {
    let (instance, adapter, device, queue) = self::init_wgpu();

    let surface_format = wgpu::TextureFormat::Bgra8Unorm;
    let target_aspect = TargetAspect::from_arg(aspect, file.is_wii);
    let (renderer, sink) =
        backend_wgpu::sink::Renderer::new(device.clone(), queue.clone(), surface_format, target_aspect, 1);

    let mut player_sink = PlayerSink::new(Box::new(sink));
    let mut playback = Playback::<SYSTEM>::new();
    playback.load_state(&file, &mut player_sink);

    let mut app = WindowedApp::<SYSTEM> {
        file,
        playback,
        sink: player_sink,
        start,
        end,
        once,
        frame_idx: start,
        finished: false,
        window: None,
        surface: None,
        renderer,
        instance,
        adapter,
        device,
        queue,
        surface_format,
    };

    let event_loop = EventLoop::new().unwrap();
    event_loop.run_app(&mut app).unwrap();
}
