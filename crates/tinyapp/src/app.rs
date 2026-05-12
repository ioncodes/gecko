use backend_wgpu::capture::{self, CaptureRequest, ScreenshotControl};
use egui::ViewportId;
use gecko::HostInput;
#[cfg(feature = "fps-counter")]
use gecko::fps::{self, FpsShared};
use gecko::hollywood::ipc::usb;
use spin_sleep::SpinSleeper;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const PACER_OFFSET: Duration = Duration::from_millis(1);
const FRAME_PERIOD_60HZ: Duration = Duration::from_nanos(16_666_667);
const CADENCE_WINDOW: usize = 16;
const CADENCE_MIN_SAMPLES: usize = 4;
const CADENCE_MIN: Duration = Duration::from_micros(500);
const CADENCE_MAX: Duration = Duration::from_millis(200);

pub struct State {
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: backend_wgpu::sink::Renderer,

    egui_ctx: egui::Context,
    egui_renderer: egui_wgpu::Renderer,
    egui_winit: egui_winit::State,
    #[cfg(not(feature = "fps-counter"))]
    last_frame: Instant,
    screenshots: ScreenshotControl,
    #[cfg(feature = "fps-counter")]
    fps_shared: FpsShared,
    intended_present_time: Option<Instant>,
    frame_period: Duration,
    pacer_offset: Duration,
    pacer_sleeper: SpinSleeper,
    last_frame_ready_at: Option<Instant>,
    recent_intervals: VecDeque<Duration>,
}

impl State {
    pub fn new(
        window: Arc<Window>,
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        surface_config: wgpu::SurfaceConfiguration,
        renderer: backend_wgpu::sink::Renderer,
        #[cfg(feature = "fps-counter")] fps_shared: FpsShared,
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
            #[cfg(not(feature = "fps-counter"))]
            last_frame: Instant::now(),
            screenshots: ScreenshotControl::new(),
            #[cfg(feature = "fps-counter")]
            fps_shared,
            intended_present_time: None,
            frame_period: FRAME_PERIOD_60HZ,
            pacer_offset: PACER_OFFSET,
            pacer_sleeper: SpinSleeper::default(),
            last_frame_ready_at: None,
            recent_intervals: VecDeque::with_capacity(CADENCE_WINDOW),
        }
    }

    pub fn observe_frame_ready(&mut self, at: Instant) {
        let Some(prev) = self.last_frame_ready_at.replace(at) else {
            return;
        };

        let Some(delta) = at.checked_duration_since(prev) else {
            return;
        };

        if !(CADENCE_MIN..=CADENCE_MAX).contains(&delta) {
            return;
        }

        if self.recent_intervals.len() == CADENCE_WINDOW {
            self.recent_intervals.pop_front();
        }

        self.recent_intervals.push_back(delta);
        let n = self.recent_intervals.len();
        if n < CADENCE_MIN_SAMPLES {
            return;
        }

        let mut buf = [Duration::ZERO; CADENCE_WINDOW];
        for (slot, d) in buf.iter_mut().zip(self.recent_intervals.iter()) {
            *slot = *d;
        }
        buf[..n].sort();

        self.frame_period = buf[n / 2];
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

    fn render(&mut self, window: &Window, waiting: bool) {
        if !waiting && let Some(intended) = self.intended_present_time {
            let target = intended + self.pacer_offset;
            let now = Instant::now();
            if target > now {
                self.pacer_sleeper.sleep(target - now);
            }
        }

        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(?e, "surface error");
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());

        #[cfg(feature = "fps-counter")]
        let (fps, native_pct) = if waiting {
            (0.0, 0.0)
        } else {
            fps::read(&self.fps_shared)
        };
        #[cfg(not(feature = "fps-counter"))]
        let fps = if waiting {
            0.0
        } else {
            let delta = self.last_frame.elapsed().as_secs_f64();
            self.last_frame = Instant::now();
            if delta > 0.0 { 1.0 / delta } else { 0.0 }
        };

        let pending_capture = if waiting {
            CaptureRequest::None
        } else {
            self.screenshots.take_pending()
        };

        if !waiting {
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
        }

        let raw_input = self.egui_winit.take_egui_input(window);
        let full_output = self.egui_ctx.run_ui(raw_input, |ui| {
            let ctx = ui.ctx().clone();
            if waiting {
                let frame = egui::Frame::window(&ctx.global_style())
                    .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 20, 220));
                egui::Window::new("wait_modal")
                    .title_bar(false)
                    .resizable(false)
                    .movable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .frame(frame)
                    .show(&ctx, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new("press space to start")
                                    .size(13.0)
                                    .color(egui::Color32::from_gray(170)),
                            );
                        });
                    });
            } else {
                let frame = egui::Frame::window(&ctx.global_style())
                    .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 20, 180));
                egui::Window::new("perf_hud")
                    .title_bar(false)
                    .resizable(false)
                    .movable(false)
                    .anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0])
                    .frame(frame)
                    .show(&ctx, |ui| {
                        #[cfg(feature = "fps-counter")]
                        ui.label(egui::RichText::new(format!("{fps:.1} FPS  {native_pct:.1}%")).monospace());
                        #[cfg(not(feature = "fps-counter"))]
                        ui.label(egui::RichText::new(format!("{fps:.1} FPS")).monospace());
                    });
            }
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
            let load = if waiting {
                wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.07,
                    g: 0.07,
                    b: 0.08,
                    a: 1.0,
                })
            } else {
                wgpu::LoadOp::Load
            };
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
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

        if !waiting {
            let now = Instant::now();
            let base = self.intended_present_time.unwrap_or(now);
            let next = (base + self.frame_period).max(now);
            self.intended_present_time = Some(next);
        }
    }
}

pub struct App {
    pub input: Arc<Mutex<HostInput>>,
    pub window: Option<Arc<Window>>,
    pub state: Option<State>,
    pub present_mode: wgpu::PresentMode,
    pub init: Option<AppInit>,
    pub _audio_stream: Option<cpal::Stream>,
    pub shutdown_requested: Arc<AtomicBool>,
    pub start_gate: Arc<AtomicBool>,
    #[cfg(feature = "fps-counter")]
    pub fps_shared: FpsShared,
}

impl App {
    fn waiting(&self) -> bool {
        !self.start_gate.load(Ordering::Acquire)
    }
}

pub struct AppInit {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub renderer: backend_wgpu::sink::Renderer,
    pub surface_format: wgpu::TextureFormat,
}

impl ApplicationHandler<crate::UserEvent> for App {
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
            #[cfg(feature = "fps-counter")]
            self.fps_shared.clone(),
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
            WindowEvent::CloseRequested => {
                self.shutdown_requested.store(true, Ordering::Relaxed);
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let (Some(state), Some(window)) = (&mut self.state, &self.window) {
                    state.resize(window, size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state.is_pressed();
                if let PhysicalKey::Code(key) = event.physical_key {
                    if self.waiting() {
                        if pressed && !event.repeat && key == KeyCode::Space {
                            self.start_gate.store(true, Ordering::Release);
                            if let Some(window) = &self.window {
                                window.request_redraw();
                            }
                        }
                        return;
                    }

                    if pressed && !event.repeat {
                        if let Some(state) = &mut self.state {
                            match key {
                                KeyCode::F11 => state.screenshots.request(CaptureRequest::FullWindow),
                                KeyCode::F12 => state.screenshots.request(CaptureRequest::GameOnly),
                                _ => {}
                            }
                        }
                    }

                    let mut input = self.input.lock().unwrap();
                    match &mut *input {
                        HostInput::Gc(pad) => crate::update_pad(pad, key, pressed),
                        HostInput::Wii {
                            wiimote_buttons,
                            nunchuk_buttons,
                            nunchuk_stick_x,
                            nunchuk_stick_y,
                            ir_pointer: _,
                        } => {
                            crate::update_wiimote_keys(wiimote_buttons, key, pressed);
                            crate::update_nunchuk_keys(nunchuk_buttons, nunchuk_stick_x, nunchuk_stick_y, key, pressed);
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state.is_pressed();
                let mask = match button {
                    MouseButton::Left => Some(usb::BTN_A),
                    MouseButton::Right => Some(usb::BTN_B),
                    _ => None,
                };
                if let Some(mask) = mask {
                    let mut input = self.input.lock().unwrap();

                    if let HostInput::Wii { wiimote_buttons, .. } = &mut *input {
                        let next = if pressed {
                            *wiimote_buttons | mask
                        } else {
                            *wiimote_buttons & !mask
                        };

                        if next != *wiimote_buttons {
                            *wiimote_buttons = next;
                            tracing::debug!(
                                mask = format!("{mask:#06x}"),
                                pressed,
                                "host mouse button mapped to Wiimote"
                            );
                        }
                    }
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
                let span_x = usb::IR_CAMERA_WIDTH as f64 * POINTER_SCALE_X;
                let span_y = usb::IR_CAMERA_HEIGHT as f64 * POINTER_SCALE_Y;
                let base_x = (usb::IR_CAMERA_WIDTH as f64 - span_x) / 2.0;
                let base_y = (usb::IR_CAMERA_HEIGHT as f64 - span_y) / 2.0 + POINTER_Y_OFFSET;
                let ir_x = (base_x + (1.0 - aim_x) * span_x) as u16;
                let ir_y = (base_y + aim_y * span_y) as u16;

                let mut input = self.input.lock().unwrap();
                if let HostInput::Wii { ir_pointer, .. } = &mut *input
                    && *ir_pointer != Some((ir_x, ir_y))
                {
                    *ir_pointer = Some((ir_x, ir_y));
                }
            }
            WindowEvent::CursorLeft { .. } => {
                let mut input = self.input.lock().unwrap();
                if let HostInput::Wii { ir_pointer, .. } = &mut *input {
                    *ir_pointer = None;
                }
            }
            WindowEvent::RedrawRequested => {
                let waiting = self.waiting();
                if let (Some(state), Some(window)) = (&mut self.state, &self.window) {
                    state.render(window, waiting);
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: crate::UserEvent) {
        match event {
            crate::UserEvent::Shutdown => {
                self.shutdown_requested.store(true, Ordering::Relaxed);
                event_loop.exit();
            }
            crate::UserEvent::FrameReady { at } => {
                if self.shutdown_requested.load(Ordering::Relaxed) {
                    event_loop.exit();
                    return;
                }

                if let Some(state) = self.state.as_mut() {
                    state.observe_frame_ready(at);
                }

                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
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
