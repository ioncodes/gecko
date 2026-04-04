use clap::Parser;
use egui::ViewportId;
use egui_plot::{Line, Plot, PlotPoints};
use gecko::flipper::si::pad::{self, PadStatus, STICK_CENTER};
use gecko::flipper::vi::regs::RefreshRate;
use gecko::gamecube::GameCube;
use image::Dol;
use image::dvd::Dvd;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const SHADER: &str = include_str!("xfb.wgsl");

struct State {
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    tex_width: u32,
    tex_height: u32,
    gx_renderer: backend_wgpu::GxRenderer,
    // egui overlay
    egui_ctx: egui::Context,
    egui_renderer: egui_wgpu::Renderer,
    egui_winit: egui_winit::State,
    fps_history: VecDeque<[f64; 2]>,
    start_time: Instant,
    last_frame: Instant,
}

impl State {
    fn new(window: Arc<Window>, emulator: &GameCube, present_mode: wgpu::PresentMode) -> Self {
        let (w, h) = emulator.frame_size();
        let (w, h) = (w as u32, h as u32);

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .unwrap();

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();

        let size = window.inner_size();
        let surface_caps = surface.get_capabilities(&adapter);

        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let (texture, bind_group) = create_xfb_texture(&device, &bind_group_layout, w, h);
        let gx_renderer = backend_wgpu::GxRenderer::new(
            &device,
            &queue,
            surface_format,
            surface_config.width,
            surface_config.height,
        );

        let egui_ctx = egui::Context::default();
        let egui_renderer = egui_wgpu::Renderer::new(&device, surface_format, egui_wgpu::RendererOptions::default());
        let egui_winit = egui_winit::State::new(egui_ctx.clone(), ViewportId::ROOT, window.as_ref(), None, None, None);

        State {
            surface,
            surface_config,
            device,
            queue,
            pipeline,
            bind_group_layout,
            texture,
            bind_group,
            tex_width: w,
            tex_height: h,
            gx_renderer,
            egui_ctx,
            egui_renderer,
            egui_winit,
            fps_history: VecDeque::new(),
            start_time: Instant::now(),
            last_frame: Instant::now(),
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
        self.gx_renderer.resize(&self.device, width, height);
    }

    fn render(&mut self, emulator: &mut GameCube, window: &Window) {
        // update FPS history
        let delta = self.last_frame.elapsed().as_secs_f64();
        self.last_frame = Instant::now();
        let fps = if delta > 0.0 { 1.0 / delta } else { 0.0 };
        let elapsed = self.start_time.elapsed().as_secs_f64();
        self.fps_history.push_back([elapsed, fps]);
        while self.fps_history.front().is_some_and(|e| elapsed - e[0] > 5.0) {
            self.fps_history.pop_front();
        }
        let native_hz = match emulator.vi.dcr.video_format().refresh_rate() {
            RefreshRate::Hz60 => 60.0_f64,
            RefreshRate::Hz50 => 50.0_f64,
        };
        let native_pct = (fps / native_hz) * 100.0;

        emulator.run_until_vsync();

        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(e) => {
                eprintln!("surface error: {e}");
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());

        if !emulator.gx.draw_commands.commands.is_empty() {
            self.render_gx(emulator, &view);
        } else {
            self.render_xfb(emulator, &view);
        }

        // egui overlay
        let raw_input = self.egui_winit.take_egui_input(window);
        let fps_points: Vec<[f64; 2]> = self.fps_history.iter().copied().collect();
        let full_output = self.egui_ctx.run_ui(raw_input, |ui| {
            let ctx = ui.ctx().clone();
            let frame =
                egui::Frame::window(&ctx.global_style()).fill(egui::Color32::from_rgba_unmultiplied(20, 20, 20, 180));
            egui::Window::new("perf_hud")
                .title_bar(false)
                .resizable(false)
                .movable(false)
                .anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0])
                .frame(frame)
                .show(&ctx, |ui| {
                    ui.label(egui::RichText::new(format!("{fps:.1} FPS  {native_pct:.1}%")).monospace());
                    Plot::new("fps_plot")
                        .height(60.0)
                        .width(180.0)
                        .show_axes(false)
                        .show_grid(false)
                        .allow_zoom(false)
                        .allow_drag(false)
                        .allow_scroll(false)
                        .show(ui, |plot_ui| {
                            plot_ui.line(
                                Line::new("fps", PlotPoints::from(fps_points.clone()))
                                    .color(egui::Color32::from_rgb(100, 200, 100)),
                            );
                        });
                });
        });

        self.egui_winit
            .handle_platform_output(window, full_output.platform_output);

        let screen_desc = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.surface_config.width, self.surface_config.height],
            pixels_per_point: window.scale_factor() as f32,
        };
        let tris = self
            .egui_ctx
            .tessellate(full_output.shapes, screen_desc.pixels_per_point);

        for (id, delta) in full_output.textures_delta.set {
            self.egui_renderer.update_texture(&self.device, &self.queue, id, &delta);
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        self.egui_renderer
            .update_buffers(&self.device, &self.queue, &mut encoder, &tris, &screen_desc);
        {
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.egui_renderer
                .render(&mut rpass.forget_lifetime(), &tris, &screen_desc);
        }
        self.queue.submit([encoder.finish()]);

        for id in full_output.textures_delta.free {
            self.egui_renderer.free_texture(&id);
        }

        frame.present();
    }

    fn render_gx(&mut self, emulator: &mut GameCube, view: &wgpu::TextureView) {
        self.gx_renderer.render(
            &self.device,
            &self.queue,
            &emulator.gx.draw_commands,
            &emulator.mmio.ram,
            view,
            self.surface_config.width,
            self.surface_config.height,
        );
        emulator.gx.draw_commands.commands.clear();
    }

    fn render_xfb(&mut self, emulator: &GameCube, view: &wgpu::TextureView) {
        let pixels = emulator.render_xfb();
        let (w, h) = emulator.frame_size();
        let (w, h) = (w as u32, h as u32);

        if (w, h) != (self.tex_width, self.tex_height) {
            let (texture, bind_group) = create_xfb_texture(&self.device, &self.bind_group_layout, w, h);
            self.texture = texture;
            self.bind_group = bind_group;
            self.tex_width = w;
            self.tex_height = h;
        }

        let rgba: Vec<u8> = pixels
            .iter()
            .flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, 0xFF])
            .collect();

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &self.bind_group, &[]);
            rpass.draw(0..3, 0..1);
        }

        self.queue.submit([encoder.finish()]);
    }
}

fn create_xfb_texture(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    w: u32,
    h: u32,
) -> (wgpu::Texture, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let view = texture.create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    (texture, bind_group)
}

struct App {
    emulator: GameCube,
    window: Option<Arc<Window>>,
    state: Option<State>,
    present_mode: wgpu::PresentMode,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("Gecko"))
                .unwrap(),
        );

        let state = State::new(window.clone(), &self.emulator, self.present_mode);
        window.request_redraw();
        self.window = Some(window);
        self.state = Some(state);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Forward events to egui first
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
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state.is_pressed();
                if let PhysicalKey::Code(key) = event.physical_key {
                    update_pad(self.emulator.primary_controller_mut(), key, pressed);
                }
            }
            WindowEvent::RedrawRequested => {
                if let (Some(state), Some(window)) = (&mut self.state, &self.window) {
                    state.render(&mut self.emulator, window);
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

#[derive(Parser)]
#[command(about = "GameCube emulator")]
struct Args {
    /// Path to the DOL file
    #[arg(long)]
    dol: Option<String>,

    /// Path to an IPL file
    #[arg(long)]
    ipl: Option<String>,

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

    let mut emulator = if let Some(ref ipl) = args.ipl {
        let ipl_data = std::fs::read(ipl).expect("failed to read IPL");
        GameCube::with_ipl(&ipl_data)
    } else if let Some(ref path) = args.dol {
        let dol_data = std::fs::read(path).expect("failed to read DOL");
        let dol = Dol::parse(dol_data);
        GameCube::with_image(&dol)
    } else {
        eprintln!("error: either --ipl or --dol must be provided");
        std::process::exit(1);
    };

    if let Some(ref iso) = args.iso {
        if args.ipl.is_none() {
            eprintln!("--iso requires --ipl");
            std::process::exit(1);
        }
        let iso_data = std::fs::read(iso).expect("failed to read ISO");
        let dvd = Dvd::parse(iso_data);
        emulator.insert_dvd(dvd);
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

    let event_loop = EventLoop::new().unwrap();
    let mut app = App {
        emulator,
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
