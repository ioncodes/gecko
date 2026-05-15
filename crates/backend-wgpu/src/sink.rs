use crate::GxRenderer;

use gecko::host::{DrawData, DrawVertex, GxAction, RenderSink};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

pub type FrameReadyCallback = Box<dyn Fn(Instant) + Send + Sync>;

/// Holds the XFB output texture view that the emu thread updates and the
/// windowing thread reads for blitting.
pub struct Shared {
    pub output: Mutex<wgpu::TextureView>,
}

/// How the XFB is fit into the present surface.
#[derive(Copy, Clone, Debug)]
pub enum TargetAspect {
    /// Fill the surface, ignoring aspect ratio.
    Stretch,
    /// Letterbox/pillarbox to the given width:height ratio.
    Ratio(f32),
}

pub struct InlineSink {
    gx: GxRenderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    shared: Arc<Shared>,
    frame_ready_cb: Arc<OnceLock<FrameReadyCallback>>,
    recycled_draw_data: Option<Box<DrawData>>,
}

impl RenderSink for InlineSink {
    fn exec(&mut self, action: GxAction) {
        self.gx.process_action(&self.device, &self.queue, &action);

        match action {
            GxAction::PresentXfb { .. } => {
                let view = self.gx.xfb_view.clone();
                *self.shared.output.lock().unwrap() = view;
                if let Some(cb) = self.frame_ready_cb.get() {
                    cb(Instant::now());
                }
            }
            GxAction::Draw(boxed) => {
                self.recycled_draw_data = Some(boxed);
            }
            _ => {}
        }
    }

    fn flush_efb_copies(&mut self, ram: &mut gecko::mmio::RamViewMut<'_>) {
        self.gx.drain_pending_writebacks(&self.device, &self.queue, ram);
    }

    fn vertex_scratch(&mut self) -> &mut Vec<DrawVertex> {
        &mut self.gx.scratch_vertices
    }

    fn take_draw_data(&mut self) -> Box<DrawData> {
        self.recycled_draw_data.take().unwrap_or_default()
    }
}

impl Drop for InlineSink {
    fn drop(&mut self) {
        self.gx.submit_pending(&self.queue);
        match self.gx.save_shader_cache() {
            Ok(n) => tracing::info!(num_variants = n, "saved shader cache"),
            Err(err) => tracing::warn!(?err, "failed to save shader cache"),
        }
        match self.gx.save_pipeline_cache() {
            Ok(n) => tracing::info!(num_pipelines = n, "saved pipeline cache"),
            Err(err) => tracing::warn!(?err, "failed to save pipeline cache"),
        }
    }
}

#[derive(Clone)]
pub struct Renderer {
    shared: Arc<Shared>,
    device: wgpu::Device,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    blit_sampler: wgpu::Sampler,
    target_aspect: TargetAspect,
    frame_ready_cb: Arc<OnceLock<FrameReadyCallback>>,
    #[cfg(feature = "renderdoc-capture")]
    renderdoc: Arc<Mutex<crate::renderdoc_capture::RenderDocCapture>>,
}

impl Renderer {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        target_aspect: TargetAspect,
    ) -> (Self, InlineSink) {
        let mut gx = GxRenderer::new(&device, &queue, surface_format);
        gx.prewarm_pipeline_cache(&device);

        // Initial shared output: the XFB view (black until first PresentXfb).
        let shared = Arc::new(Shared {
            output: Mutex::new(gx.xfb_view.clone()),
        });

        // Build the blit pipeline.
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("efb_blit_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/efb_blit.wgsl").into()),
        });
        let blit_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
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
        let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit_layout"),
            bind_group_layouts: &[&blit_bind_group_layout],
            immediate_size: 0,
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit_pipeline"),
            layout: Some(&blit_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blit_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let frame_ready_cb: Arc<OnceLock<FrameReadyCallback>> = Arc::new(OnceLock::new());

        let sink = InlineSink {
            gx,
            device: device.clone(),
            queue,
            shared: shared.clone(),
            frame_ready_cb: frame_ready_cb.clone(),
            recycled_draw_data: None,
        };

        let renderer = Renderer {
            shared,
            device,
            blit_pipeline,
            blit_bind_group_layout,
            blit_sampler,
            target_aspect,
            frame_ready_cb,
            #[cfg(feature = "renderdoc-capture")]
            renderdoc: Arc::new(Mutex::new(crate::renderdoc_capture::RenderDocCapture::new())),
        };

        (renderer, sink)
    }

    #[cfg(feature = "renderdoc-capture")]
    pub fn begin_renderdoc_emulated_frame(&self) {
        if let Ok(mut rd) = self.renderdoc.lock() {
            rd.begin_emulated_frame();
        }
    }

    #[cfg(feature = "renderdoc-capture")]
    pub fn end_renderdoc_emulated_frame(&self) {
        if let Ok(mut rd) = self.renderdoc.lock() {
            rd.end_emulated_frame();
        }
    }

    #[cfg(feature = "renderdoc-capture")]
    pub fn capture_next_renderdoc_emulated_frame(&self) {
        if let Ok(mut rd) = self.renderdoc.lock() {
            rd.request_next_emulated_frame();
        }
    }

    #[cfg(feature = "renderdoc-capture")]
    pub fn start_renderdoc_frame_capture(&self) {
        if let Ok(mut rd) = self.renderdoc.lock() {
            rd.start_frame_capture();
        }
    }

    #[cfg(feature = "renderdoc-capture")]
    pub fn end_renderdoc_frame_capture(&self) {
        if let Ok(mut rd) = self.renderdoc.lock() {
            rd.end_frame_capture();
        }
    }

    #[cfg(feature = "renderdoc-capture")]
    pub fn trigger_renderdoc_capture(&self) {
        if let Ok(mut rd) = self.renderdoc.lock() {
            rd.trigger_capture();
        }
    }

    pub fn set_frame_ready_callback<F>(&self, cb: F)
    where
        F: Fn(Instant) + Send + Sync + 'static,
    {
        let _ = self.frame_ready_cb.set(Box::new(cb));
    }

    pub fn target_aspect(&self) -> TargetAspect {
        self.target_aspect
    }

    /// Blit the latest XFB output to the given render target. `target_size`
    /// is the destination view's pixel size; used to letterbox/pillarbox the
    /// XFB to `self.target_aspect`. Called by the windowing thread on each
    /// redraw.
    pub fn blit(&self, queue: &wgpu::Queue, target: &wgpu::TextureView, target_size: (u32, u32)) {
        let output = self.shared.output.lock().unwrap();
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit_bg"),
            layout: &self.blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&output),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.blit_sampler),
                },
            ],
        });
        drop(output);

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("xfb_blit_encoder"),
        });
        encoder.push_debug_group("XFB Blit To Surface");
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xfb_blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
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
            let (vx, vy, vw, vh) = self::viewport_for_aspect(target_size, self.target_aspect);
            rpass.set_viewport(vx, vy, vw, vh, 0.0, 1.0);
            rpass.set_pipeline(&self.blit_pipeline);
            rpass.set_bind_group(0, &bind_group, &[]);
            rpass.insert_debug_marker("Draw fullscreen XFB blit");
            rpass.draw(0..3, 0..1);
        }
        encoder.pop_debug_group();
        queue.submit([encoder.finish()]);
    }
}

/// Snap a requested window size to the largest rectangle with
/// `target_aspect` that fits inside it. For Stretch this returns the input
/// unchanged. The window code calls this on resize so the OS window itself
/// matches the target AR (no letterbox bars in the surface).
pub fn snap_size_to_aspect(requested: (u32, u32), target_aspect: TargetAspect) -> (u32, u32) {
    let (w, h) = (requested.0.max(1), requested.1.max(1));
    match target_aspect {
        TargetAspect::Stretch => (w, h),
        TargetAspect::Ratio(ar) => {
            let surface_ar = w as f32 / h as f32;
            if surface_ar > ar {
                let new_w = ((h as f32) * ar).round() as u32;
                (new_w.max(1), h)
            } else {
                let new_h = ((w as f32) / ar).round() as u32;
                (w, new_h.max(1))
            }
        }
    }
}

/// Compute the (x, y, w, h) viewport rect that fits `target_aspect` inside
/// `target_size`. Stretch returns the full surface; Ratio centers a maximal
/// sub-rect with the requested width:height, leaving the cleared surface
/// visible as letterbox/pillarbox bars.
#[inline(always)]
pub(crate) fn viewport_for_aspect(target_size: (u32, u32), target_aspect: TargetAspect) -> (f32, f32, f32, f32) {
    let (w, h) = (target_size.0.max(1) as f32, target_size.1.max(1) as f32);
    match target_aspect {
        TargetAspect::Stretch => (0.0, 0.0, w, h),
        TargetAspect::Ratio(ar) => {
            let surface_ar = w / h;
            if surface_ar > ar {
                let vw = h * ar;
                ((w - vw) * 0.5, 0.0, vw, h)
            } else {
                let vh = w / ar;
                (0.0, (h - vh) * 0.5, w, vh)
            }
        }
    }
}
