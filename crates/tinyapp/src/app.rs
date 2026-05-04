use crate::thread::FrameMessage;
use backend_wgpu::capture::{self, CaptureRequest, ScreenshotControl};
use crossbeam_channel::Receiver;
use egui::ViewportId;
use gecko::flipper::si::pad::PadStatus;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

pub struct State {
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: backend_wgpu::sink::Renderer,

    egui_ctx: egui::Context,
    egui_renderer: egui_wgpu::Renderer,
    egui_winit: egui_winit::State,
    fps_history: VecDeque<[f64; 2]>,
    start_time: Instant,
    last_frame: Instant,
    last_native_hz: f64,
    screenshots: ScreenshotControl,
}

impl State {
    pub fn new(
        window: Arc<Window>,
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        surface_config: wgpu::SurfaceConfiguration,
        renderer: backend_wgpu::sink::Renderer,
    ) -> Self {
        let egui_ctx = egui::Context::default();
        let egui_renderer =
            egui_wgpu::Renderer::new(&device, surface_config.format, egui_wgpu::RendererOptions::default());
        let egui_winit = egui_winit::State::new(egui_ctx.clone(), ViewportId::ROOT, window.as_ref(), None, None, None);

        State {
            surface,
            surface_config,
            device,
            queue,
            renderer,
            egui_ctx,
            egui_renderer,
            egui_winit,
            fps_history: VecDeque::new(),
            start_time: Instant::now(),
            last_frame: Instant::now(),
            last_native_hz: 60.0,
            screenshots: ScreenshotControl::new(),
        }
    }

    fn resize(&mut self, window: &Window, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        // Snap so the OS window matches the target aspect ratio (no bars).
        // Skip when already matching to avoid feedback from the next Resized
        // event triggered by request_inner_size.
        let (sw, sh) = backend_wgpu::sink::snap_size_to_aspect((width, height), self.renderer.target_aspect());
        if (sw, sh) != (width, height) {
            let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(sw, sh));
        }
        self.surface_config.width = sw;
        self.surface_config.height = sh;
        self.surface.configure(&self.device, &self.surface_config);
    }

    fn render(&mut self, frame_rx: &Receiver<FrameMessage>, window: &Window) {
        let mut msg: Option<FrameMessage> = None;
        while let Ok(newer) = frame_rx.try_recv() {
            msg = Some(newer);
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
        let pending_capture = self.screenshots.take_pending();

        // Blit the latest XFB output from the renderer worker to the swapchain.
        self.renderer.blit(
            &self.queue,
            &view,
            (self.surface_config.width, self.surface_config.height),
        );

        // Capture the game-only screen before the egui overlay is drawn. Reads
        // the swapchain instead of the XFB directly so the blit's fullscreen
        // sample resolves any partial-update state in the XFB accumulator.
        if let CaptureRequest::GameOnly = pending_capture
            && let Some(cap) = capture::capture_texture(&self.device, &self.queue, &frame.texture)
        {
            capture::save_png_async(capture::timestamped_path("game"), cap, true);
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

        if let CaptureRequest::FullWindow = pending_capture
            && let Some(cap) = capture::capture_texture(&self.device, &self.queue, &frame.texture)
        {
            capture::save_png_async(capture::timestamped_path("full"), cap, true);
        }

        frame.present();
    }
}

pub struct App {
    pub frame_rx: Receiver<FrameMessage>,
    pub input: Arc<Mutex<PadStatus>>,
    pub window: Option<Arc<Window>>,
    pub state: Option<State>,
    pub present_mode: wgpu::PresentMode,
    pub init: Option<AppInit>,
    pub _audio_stream: Option<cpal::Stream>,
    pub shutdown_requested: Arc<AtomicBool>,
}

pub struct AppInit {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub renderer: backend_wgpu::sink::Renderer,
    pub surface_format: wgpu::TextureFormat,
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
                        .with_title("Tiny Gecko")
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

        let state = State::new(
            window.clone(),
            init.device,
            init.queue,
            surface,
            surface_config,
            init.renderer,
        );
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
                                KeyCode::F11 => state.screenshots.request(CaptureRequest::FullWindow),
                                KeyCode::F12 => state.screenshots.request(CaptureRequest::GameOnly),
                                _ => {}
                            }
                        }
                    }
                    let mut pad = self.input.lock().unwrap();
                    crate::update_pad(&mut pad, key, pressed);
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

    fn user_event(&mut self, event_loop: &ActiveEventLoop, _event: ()) {
        if self.shutdown_requested.load(Ordering::Relaxed) {
            event_loop.exit();
            return;
        }

        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Pick a sensible initial window size for the given target aspect.
fn initial_window_size(target_aspect: backend_wgpu::sink::TargetAspect) -> (u32, u32) {
    use backend_wgpu::sink::TargetAspect;
    match target_aspect {
        TargetAspect::Stretch => (1280, 720),
        TargetAspect::Ratio(ar) => {
            // 720 lines tall, width derived from AR.
            let h: u32 = 720;
            let w = ((h as f32) * ar).round() as u32;
            (w, h)
        }
    }
}
