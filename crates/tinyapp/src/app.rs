use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crossbeam_channel::Receiver;
use egui::ViewportId;
use gecko::flipper::si::pad::PadStatus;
use gecko::host::GxAction;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

use crate::thread::FrameMessage;
use crate::update_pad;

pub struct State {
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    gx_renderer: backend_wgpu::GxRenderer,
    action_rx: backend_wgpu::sink::ActionReceiver,
    action_buf: Vec<GxAction>,
    // egui overlay
    egui_ctx: egui::Context,
    egui_renderer: egui_wgpu::Renderer,
    egui_winit: egui_winit::State,
    fps_history: VecDeque<[f64; 2]>,
    start_time: Instant,
    last_frame: Instant,
    last_native_hz: f64,
}

impl State {
    pub fn new(
        window: Arc<Window>,
        present_mode: wgpu::PresentMode,
        action_rx: backend_wgpu::sink::ActionReceiver,
    ) -> Self {
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

        let gx_renderer = backend_wgpu::GxRenderer::new(&device, &queue, wgpu::TextureFormat::Bgra8Unorm);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
            format: wgpu::TextureFormat::Bgra8Unorm,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let egui_ctx = egui::Context::default();
        let egui_renderer = egui_wgpu::Renderer::new(
            &device,
            wgpu::TextureFormat::Bgra8Unorm,
            egui_wgpu::RendererOptions::default(),
        );
        let egui_winit = egui_winit::State::new(egui_ctx.clone(), ViewportId::ROOT, window.as_ref(), None, None, None);

        State {
            surface,
            surface_config,
            device,
            queue,
            gx_renderer,
            action_rx,
            action_buf: Vec::new(),
            egui_ctx,
            egui_renderer,
            egui_winit,
            fps_history: VecDeque::new(),
            start_time: Instant::now(),
            last_frame: Instant::now(),
            last_native_hz: 60.0,
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    fn render(&mut self, frame_rx: &Receiver<FrameMessage>, window: &Window) {
        let mut msg: Option<FrameMessage> = None;
        while let Ok(newer) = frame_rx.try_recv() {
            msg = Some(newer); // drain
        }

        let Some(msg) = msg else { return };

        // FPS bookkeeping
        let delta = self.last_frame.elapsed().as_secs_f64();
        self.last_frame = Instant::now();
        let fps = if delta > 0.0 { 1.0 / delta } else { 0.0 };
        let elapsed = self.start_time.elapsed().as_secs_f64();
        self.fps_history.push_back([elapsed, fps]);
        while self.fps_history.front().is_some_and(|e| elapsed - e[0] > 5.0) {
            self.fps_history.pop_front();
        }
        self.last_native_hz = msg.native_hz;
        let native_pct = (fps / self.last_native_hz) * 100.0;

        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(e) => {
                eprintln!("surface error: {e}");
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());

        self.action_buf.clear();
        self.action_rx.drain(&mut self.action_buf);

        // Drive the GX renderer with any queued actions (draws into its EFB
        // and updates the snapshot on CopyEfb).
        if !self.action_buf.is_empty() {
            self.gx_renderer
                .render_actions(&self.device, &self.queue, &self.action_buf);
        }

        // Blit the latest EFB snapshot to the swapchain.
        self.gx_renderer.blit_to_target(&self.device, &self.queue, &view);

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
                    egui_plot::Plot::new("fps_plot")
                        .height(60.0)
                        .width(180.0)
                        .show_axes(false)
                        .show_grid(false)
                        .allow_zoom(false)
                        .allow_drag(false)
                        .allow_scroll(false)
                        .show(ui, |plot_ui| {
                            plot_ui.line(
                                egui_plot::Line::new("fps", egui_plot::PlotPoints::from(fps_points.clone()))
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
}

// ---------------------------------------------------------------------------
// Winit application handler
// ---------------------------------------------------------------------------

pub struct App {
    pub frame_rx: Receiver<FrameMessage>,
    pub action_rx: Option<backend_wgpu::sink::ActionReceiver>,
    pub input: Arc<Mutex<PadStatus>>,
    pub window: Option<Arc<Window>>,
    pub state: Option<State>,
    pub present_mode: wgpu::PresentMode,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("Gecko"))
                .unwrap(),
        );

        let action_rx = self.action_rx.take().expect("action_rx consumed twice");
        let state = State::new(window.clone(), self.present_mode, action_rx);
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
                    let mut pad = self.input.lock().unwrap();
                    update_pad(&mut pad, key, pressed);
                }
            }
            WindowEvent::RedrawRequested => {
                if let (Some(state), Some(window)) = (&mut self.state, &self.window) {
                    state.render(&self.frame_rx, window);
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
