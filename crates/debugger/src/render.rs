use dbglib::EmulatorState;
use egui::ViewportId;
use gecko::gamecube::GameCube;
use std::sync::Arc;
use winit::window::Window;

use crate::debugger::DebuggerUi;

const SHADER: &str = include_str!("xfb.wgsl");

pub struct RenderState {
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
    pub egui_ctx: egui::Context,
    pub egui_renderer: egui_wgpu::Renderer,
    pub egui_winit: egui_winit::State,
}

impl RenderState {
    pub fn new(window: Arc<Window>, emulator: &GameCube, present_mode: wgpu::PresentMode) -> Self {
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
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        egui_ctx.set_fonts(fonts);
        egui_ctx.global_style_mut(|style| {
            let f = &style.visuals.window_fill;
            style.visuals.window_fill = egui::Color32::from_rgba_unmultiplied(f.r(), f.g(), f.b(), 240);
        });
        let egui_renderer = egui_wgpu::Renderer::new(&device, surface_format, egui_wgpu::RendererOptions::default());
        let egui_winit = egui_winit::State::new(egui_ctx.clone(), ViewportId::ROOT, window.as_ref(), None, None, None);

        RenderState {
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
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
        self.gx_renderer.resize(&self.device, width, height);
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
        if debugger_ui.show_lua {
            if let Some(ref mut host) = emulator.script_host {
                debugger_ui.lua_log.extend(host.drain_logs());
            }
        }

        // Load a new Lua script if requested
        if debugger_ui.lua_load_pending {
            debugger_ui.lua_load_pending = false;
            match scripting::LuaScriptHost::from_source("editor", &debugger_ui.lua_source) {
                Ok(host) => {
                    debugger_ui.lua_log.push("[lua] script loaded".to_string());
                    emulator.set_script_host(Box::new(host));
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

        let used_gx = !emulator.gx.draw_commands.commands.is_empty();
        if used_gx {
            self.render_gx(emulator, &view);
        } else {
            self.render_xfb(emulator, &view);
        }

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
                        if ui
                            .add_enabled(
                                is_paused,
                                egui::Button::new(format!("{} Step DSP", icons::SKIP_FORWARD)),
                            )
                            .clicked()
                        {
                            debugger_ui.debugger.set_state(EmulatorState::StepDsp);
                            ui.close();
                        }
                        if ui.button(format!("{} Run Until VSync", icons::FAST_FORWARD)).clicked() {
                            debugger_ui.debugger.set_state(EmulatorState::RunUntilVsync);
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
                let tracing = debugger_ui.debugger.is_tracing();
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
                );
                debugger_ui.debugger.set_state(state);
                if start_trace {
                    debugger_ui.start_trace();
                }
                if stop_trace {
                    debugger_ui.stop_trace();
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

        // Clear after egui so the GX state window can inspect the draw calls from this frame.
        if used_gx {
            emulator.gx.draw_commands.commands.clear();
        }
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
