use backend_wgpu::capture::{self, CaptureRequest, ScreenshotControl};
use dbglib::EmulatorState;
use egui::ViewportId;
use gecko::system::{System, SystemId};
use std::sync::Arc;
use winit::window::Window;

use crate::debugger::DebuggerUi;

pub struct RenderState {
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: backend_wgpu::sink::Renderer,
    pub egui_ctx: egui::Context,
    pub egui_renderer: egui_wgpu::Renderer,
    pub egui_winit: egui_winit::State,
    screenshots: ScreenshotControl,
}

impl RenderState {
    pub fn new(
        window: Arc<Window>,
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        surface_config: wgpu::SurfaceConfiguration,
        renderer: backend_wgpu::sink::Renderer,
        _present_mode: wgpu::PresentMode,
    ) -> Self {
        let egui_ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        fonts
            .font_data
            .insert("phosphor-fill".into(), egui_phosphor::Variant::Fill.font_data().into());
        fonts.families.insert(
            egui::FontFamily::Name("phosphor-fill".into()),
            vec!["phosphor-fill".into()],
        );
        egui_ctx.set_fonts(fonts);
        egui_ctx.global_style_mut(|style| {
            let f = &style.visuals.window_fill;
            style.visuals.window_fill = egui::Color32::from_rgba_unmultiplied(f.r(), f.g(), f.b(), 240);
        });
        let egui_renderer =
            egui_wgpu::Renderer::new(&device, surface_config.format, egui_wgpu::RendererOptions::default());
        let egui_winit = egui_winit::State::new(egui_ctx.clone(), ViewportId::ROOT, window.as_ref(), None, None, None);

        RenderState {
            surface,
            surface_config,
            device,
            queue,
            renderer,
            egui_ctx,
            egui_renderer,
            egui_winit,
            screenshots: ScreenshotControl::new(),
        }
    }

    pub fn request_screenshot(&mut self, req: CaptureRequest) {
        self.screenshots.request(req);
    }

    #[cfg(feature = "renderdoc-capture")]
    pub fn request_renderdoc_capture(&self) {
        self.renderer.capture_next_renderdoc_emulated_frame();
    }

    pub fn resize(&mut self, window: &Window, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        let (sw, sh) = backend_wgpu::sink::snap_size_to_aspect((width, height), self.renderer.target_aspect());
        if (sw, sh) != (width, height) {
            let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(sw, sh));
        }
        self.surface_config.width = sw;
        self.surface_config.height = sh;
        self.surface.configure(&self.device, &self.surface_config);
    }

    pub fn render<const SYSTEM: SystemId>(
        &mut self,
        emulator: &mut System<SYSTEM>,
        debugger_ui: &mut DebuggerUi,
        window: &Window,
    ) {
        if let Some(open) = debugger_ui.dvd_cover_open.take() {
            if open {
                emulator.open_cover();
            } else {
                emulator.close_cover();
            }
        }

        // Drain Lua log messages from the script host. `drain_logs` is on the
        // generic `Host` trait so this works regardless of system. Loading a
        // new Lua script is handled by the App, since `LuaHost` only impls
        // `Host<{ GC }>`.
        if debugger_ui.show_lua
            && let Some(ref mut host) = emulator.hook_host
        {
            debugger_ui.lua_log.extend(host.drain_logs());
        }

        debugger_ui.debugger.tick(emulator);

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            status => {
                eprintln!("surface error: {status:?}");
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

        let cpu = &emulator.gekko;
        let mmio = &emulator.mmio;
        let gx = &emulator.gx;

        let raw_input = self.egui_winit.take_egui_input(window);
        let full_output = self.egui_ctx.run_ui(raw_input, |ui| {
            let ctx = ui.ctx().clone();

            egui::Panel::top("menu_bar").show_inside(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button("Emulator", |ui| {
                        let is_paused = debugger_ui.debugger.state() == EmulatorState::Paused;
                        let is_running = debugger_ui.debugger.state() == EmulatorState::Running;

                        use egui_phosphor::regular as icons;
                        if ui
                            .add_enabled(is_paused, egui::Button::new(format!("{} Continue", icons::PLAY)))
                            .clicked()
                        {
                            debugger_ui.debugger.set_state(EmulatorState::Running);
                            ui.close();
                        }
                        if ui
                            .add_enabled(is_running, egui::Button::new(format!("{} Pause", icons::PAUSE)))
                            .clicked()
                        {
                            debugger_ui.debugger.set_state(EmulatorState::Paused);
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                is_paused,
                                egui::Button::new(format!("{} Step CPU", icons::SKIP_FORWARD)),
                            )
                            .clicked()
                        {
                            debugger_ui.debugger.set_state(EmulatorState::Step);
                            ui.close();
                        }
                        if ui.button(format!("{} Run Until VSync", icons::FAST_FORWARD)).clicked() {
                            debugger_ui.debugger.set_state(EmulatorState::RunUntilVsync);
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                is_paused,
                                egui::Button::new(format!("{} Run Until DSP", icons::FAST_FORWARD)),
                            )
                            .clicked()
                        {
                            debugger_ui.debugger.set_state(EmulatorState::RunUntilDsp);
                            ui.close();
                        }

                        ui.separator();

                        if ui.button(format!("{} Save State (F5)", icons::FLOPPY_DISK)).clicked() {
                            debugger_ui.save_state_requested = true;
                            ui.close();
                        }
                        if ui.button(format!("{} Load State (F7)", icons::FOLDER_OPEN)).clicked() {
                            debugger_ui.load_state_requested = true;
                            ui.close();
                        }

                        if !debugger_ui.savestate_last_result.is_empty() {
                            ui.separator();
                            ui.label(debugger_ui.savestate_last_result.as_str());
                        }
                    });

                    ui.menu_button("Windows", |ui| {
                        ui.checkbox(&mut debugger_ui.show_cpu, "CPU");
                        ui.checkbox(&mut debugger_ui.show_callstack, "Call Stack");
                        ui.checkbox(&mut debugger_ui.show_dsp, "DSP");
                        ui.checkbox(&mut debugger_ui.show_gx_state, "GX");
                        ui.checkbox(&mut debugger_ui.show_mmio, "MMIO");
                        ui.checkbox(&mut debugger_ui.show_dvd, "DVD");
                        ui.checkbox(&mut debugger_ui.show_exi, "EXI");
                        ui.checkbox(&mut debugger_ui.show_irqs, "IRQ");
                        ui.checkbox(&mut debugger_ui.show_controls, "Controls");
                        ui.checkbox(&mut debugger_ui.show_breakpoints, "Breakpoints");
                        ui.checkbox(&mut debugger_ui.show_fifo_recorder, "FIFO Recorder");
                        ui.checkbox(&mut debugger_ui.show_lua, "Lua");
                    });

                    ui.menu_button("Help", |ui| {
                        ui.checkbox(&mut debugger_ui.show_about, "About");
                    });
                });
            });

            if debugger_ui.show_cpu {
                dbglib::windows::cpu::show_cpu(
                    &ctx,
                    &mut debugger_ui.show_cpu,
                    cpu,
                    mmio,
                    debugger_ui.symbols.as_ref(),
                    debugger_ui.debugger.breakpoints(),
                );
            }
            if debugger_ui.show_callstack {
                dbglib::windows::callstack::show_callstack(
                    &ctx,
                    &mut debugger_ui.show_callstack,
                    cpu,
                    mmio,
                    debugger_ui.symbols.as_ref(),
                );
            }
            if debugger_ui.show_dsp {
                dbglib::windows::dsp::show_dsp(&ctx, &mut debugger_ui.show_dsp, &emulator.dsp);
            }
            if debugger_ui.show_controls {
                let mut start_trace = false;
                let mut stop_trace = false;
                let mut start_dsp_trace = false;
                let mut stop_dsp_trace = false;
                let tracing = debugger_ui.debugger.is_tracing();
                let dsp_tracing = debugger_ui.debugger.is_dsp_tracing();
                let mut state = debugger_ui.debugger.state();
                dbglib::windows::controls::show_controls(
                    &ctx,
                    &mut debugger_ui.show_controls,
                    &mut state,
                    &mut debugger_ui.run_until_addr_input,
                    &mut debugger_ui.dvd_cover_open,
                    tracing,
                    &mut start_trace,
                    &mut stop_trace,
                    dsp_tracing,
                    &mut start_dsp_trace,
                    &mut stop_dsp_trace,
                );
                debugger_ui.debugger.set_state(state);
                if start_trace {
                    debugger_ui.start_trace();
                }
                if stop_trace {
                    debugger_ui.stop_trace();
                }
                if start_dsp_trace {
                    debugger_ui.start_dsp_trace();
                }
                if stop_dsp_trace {
                    debugger_ui.stop_dsp_trace();
                }
            }
            if debugger_ui.show_gx_state {
                dbglib::windows::gx::show_gx(
                    &ctx,
                    &mut debugger_ui.show_gx_state,
                    gx,
                    mmio,
                    &mut debugger_ui.gx_invalidate_requested,
                    &mut debugger_ui.gx_dump_requested,
                    &mut debugger_ui.freecam,
                );
            }
            debugger_ui.freecam.update_input(&ctx);
            if debugger_ui.show_mmio {
                dbglib::windows::mmio::show_mmio(
                    &ctx,
                    &mut debugger_ui.show_mmio,
                    &mut debugger_ui.memory_base,
                    &mut debugger_ui.memory_addr_input,
                    mmio,
                );
            }
            if debugger_ui.show_dvd {
                dbglib::windows::dvd::show_dvd(&ctx, &mut debugger_ui.show_dvd, &emulator.di);
            }
            if debugger_ui.show_exi {
                dbglib::windows::exi::show_exi(&ctx, &mut debugger_ui.show_exi, &emulator.exi);
            }
            if debugger_ui.show_irqs {
                dbglib::windows::irq::show_irq(&ctx, &mut debugger_ui.show_irqs, &emulator.gekko, &emulator.pi);
            }
            if debugger_ui.show_breakpoints {
                dbglib::windows::breakpoints::show_breakpoints(
                    &ctx,
                    &mut debugger_ui.show_breakpoints,
                    &mut debugger_ui.debugger,
                    &mut debugger_ui.breakpoint_addr_input,
                );
            }
            if debugger_ui.show_fifo_recorder {
                let mut action = None;
                dbglib::windows::fifo::show_fifo_recorder(
                    &ctx,
                    &mut debugger_ui.show_fifo_recorder,
                    gx.recorder.as_deref(),
                    &mut debugger_ui.fifo_path_input,
                    &debugger_ui.fifo_last_result,
                    &mut action,
                );
                debugger_ui.fifo_action = debugger_ui.fifo_action.or(action);
            }
            if debugger_ui.show_about {
                dbglib::windows::about::show_about(&ctx, &mut debugger_ui.show_about);
            }
            if debugger_ui.show_lua {
                let mut load_script = false;
                let mut clear_log = false;
                dbglib::windows::lua::show_lua(
                    &ctx,
                    &mut debugger_ui.show_lua,
                    &mut debugger_ui.lua_source,
                    &debugger_ui.lua_log,
                    &mut load_script,
                    &mut clear_log,
                );
                if clear_log {
                    debugger_ui.lua_log.clear();
                }
                if load_script {
                    debugger_ui.lua_load_pending = true;
                }
            }
        });

        match debugger_ui.fifo_action.take() {
            Some(dbglib::windows::fifo::FifoRecorderAction::Start) => {
                emulator.gx.recorder = Some(Box::new(gecko::flipper::gx::recorder::FifoRecorder::new()));
                debugger_ui.fifo_last_result = "recording until stopped".to_string();
            }
            Some(dbglib::windows::fifo::FifoRecorderAction::Stop) => {
                if let Some(rec) = emulator.gx.recorder.as_deref_mut() {
                    rec.request_stop();
                }
            }
            Some(dbglib::windows::fifo::FifoRecorderAction::Cancel) => {
                emulator.gx.recorder = None;
                debugger_ui.fifo_last_result = "recording cancelled".to_string();
            }
            None => {}
        }

        if emulator
            .gx
            .recorder
            .as_deref()
            .is_some_and(|r| r.state() == gecko::flipper::gx::recorder::RecorderState::Done)
        {
            let rec = emulator.gx.recorder.take().unwrap();
            let file = rec.into_file();
            let frame_count = file.frames.len();
            if frame_count == 0 {
                debugger_ui.fifo_last_result = "no frames recorded, nothing saved".to_string();
            } else {
                let path = std::path::PathBuf::from(debugger_ui.fifo_path_input.trim());
                debugger_ui.fifo_last_result = match file.save(&path) {
                    Ok(()) => format!("saved {frame_count} frames to {}", path.display()),
                    Err(e) => format!("save failed: {e}"),
                };
            }
        }

        if std::mem::take(&mut debugger_ui.save_state_requested) {
            debugger_ui.savestate_last_result = match emulator.save_state_to_file(&debugger_ui.savestate_path) {
                Ok(()) => format!("state saved to {}", debugger_ui.savestate_path.display()),
                Err(err) => format!("state save failed: {err}"),
            };
        }

        if std::mem::take(&mut debugger_ui.load_state_requested) {
            debugger_ui.savestate_last_result = match emulator.load_state_from_file(&debugger_ui.savestate_path) {
                Ok(()) => format!("state loaded from {}", debugger_ui.savestate_path.display()),
                Err(err) => format!("state load failed: {err}"),
            };
        }

        if std::mem::take(&mut debugger_ui.gx_invalidate_requested) {
            emulator.gx.texture_hashes.clear();
            emulator.render_sink.exec(gecko::host::GxAction::InvalidateCaches);
        }

        if let Some((matrix, enabled)) = debugger_ui.freecam.take_action() {
            emulator
                .render_sink
                .exec(gecko::host::GxAction::SetFreelook { matrix, enabled });
        }
        #[cfg(not(target_arch = "wasm32"))]
        if std::mem::take(&mut debugger_ui.gx_dump_requested) {
            emulator.render_sink.exec(gecko::host::GxAction::DumpTextures {
                dir: std::path::PathBuf::from("textures"),
            });
        }

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

        #[cfg(feature = "renderdoc-capture")]
        self.submit_swapchain_present_marker();
        frame.present();
    }

    #[cfg(feature = "renderdoc-capture")]
    fn submit_swapchain_present_marker(&self) {
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("swapchain_present_marker_encoder"),
        });
        encoder.push_debug_group("Swapchain Present");
        encoder.insert_debug_marker("SurfaceTexture::present follows this marker");
        encoder.pop_debug_group();
        self.queue.submit([encoder.finish()]);
    }
}
