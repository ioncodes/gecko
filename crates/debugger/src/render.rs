use dbglib::EmulatorState;
use egui::ViewportId;
use gecko::gamecube::GameCube;
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
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    pub fn render(&mut self, emulator: &mut GameCube, debugger_ui: &mut DebuggerUi, window: &Window) {
        if let Some(open) = debugger_ui.dvd_cover_open.take() {
            if open {
                emulator.open_cover();
            } else {
                emulator.close_cover();
            }
        }

        // Drain Lua log messages from the script host
        if debugger_ui.show_lua
            && let Some(ref mut host) = emulator.hook_host
        {
            debugger_ui.lua_log.extend(host.drain_logs());
        }

        // Load a new Lua script if requested
        if debugger_ui.lua_load_pending {
            debugger_ui.lua_load_pending = false;
            match scripting::LuaHost::from_source("editor", &debugger_ui.lua_source) {
                Ok(host) => {
                    debugger_ui.lua_log.push("[lua] script loaded".to_string());
                    emulator.set_hook_host(Box::new(host));
                }
                Err(err) => {
                    debugger_ui.lua_log.push(format!("[lua] error: {err}"));
                }
            }
        }

        debugger_ui.debugger.tick(emulator);

        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(e) => {
                eprintln!("surface error: {e}");
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());

        // Blit the latest XFB output from the renderer worker to the swapchain.
        self.renderer.blit(&self.queue, &view);

        let cpu = &emulator.cpu;
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
                        ui.checkbox(&mut debugger_ui.show_lua, "Lua");
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
                dbglib::windows::gx::show_gx(&ctx, &mut debugger_ui.show_gx_state, gx, mmio);
            }
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
                dbglib::windows::irq::show_irq(&ctx, &mut debugger_ui.show_irqs, &emulator.cpu, &emulator.pi);
            }
            if debugger_ui.show_breakpoints {
                dbglib::windows::breakpoints::show_breakpoints(
                    &ctx,
                    &mut debugger_ui.show_breakpoints,
                    &mut debugger_ui.debugger,
                    &mut debugger_ui.breakpoint_addr_input,
                );
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
