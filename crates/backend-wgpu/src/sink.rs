use gecko::host::GxAction;

#[cfg(not(target_arch = "wasm32"))]
use crate::GxRenderer;
#[cfg(not(target_arch = "wasm32"))]
use gecko::host::{DrawData, DrawVertex, RenderSink};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(not(target_arch = "wasm32"))]
pub type FrameReadyCallback = Box<dyn Fn(Instant) + Send + Sync>;

#[cfg(not(target_arch = "wasm32"))]
const WORK_QUEUE_LIMIT: usize = 4096;

/// Holds the XFB output texture that the render worker updates and the
/// windowing thread reads for blitting and screenshots.
#[cfg(not(target_arch = "wasm32"))]
pub struct Shared {
    pub output: Mutex<wgpu::Texture>,
}

/// How the XFB is fit into the present surface.
#[derive(Copy, Clone, Debug)]
pub enum TargetAspect {
    /// Fill the surface, ignoring aspect ratio.
    Stretch,
    /// Letterbox/pillarbox to the given width:height ratio.
    Ratio(f32),
}

impl TargetAspect {
    pub fn from_arg(arg: &str, is_wii: bool) -> Self {
        match arg {
            "auto" => {
                if is_wii {
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
}

#[cfg(not(target_arch = "wasm32"))]
pub struct ThreadedSink {
    work_tx: crossbeam_channel::Sender<WorkerCommand>,
    recycled_draw_data_rx: crossbeam_channel::Receiver<Box<DrawData>>,
    worker_thread: Option<JoinHandle<()>>,
    scratch: Vec<DrawVertex>,
    scratch_sent_len: usize,
}

#[cfg(not(target_arch = "wasm32"))]
struct RenderWorker {
    gx: GxRenderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    shared: Arc<Shared>,
    frame_ready_cb: Arc<OnceLock<FrameReadyCallback>>,
    recycled_draw_data_tx: crossbeam_channel::Sender<Box<DrawData>>,
    #[cfg(feature = "renderdoc-capture")]
    renderdoc: Arc<Mutex<crate::renderdoc_capture::RenderDocCapture>>,
}

#[cfg(not(target_arch = "wasm32"))]
struct ActionMessage {
    action: GxAction,
    vertices: Vec<DrawVertex>,
}

#[cfg(not(target_arch = "wasm32"))]
struct EfbDrainRequest {
    mem1_addr: usize,
    mem1_len: usize,
    mem2_addr: usize,
    mem2_len: usize,
    done_tx: crossbeam_channel::Sender<()>,
}

#[cfg(not(target_arch = "wasm32"))]
enum WorkerCommand {
    Action(ActionMessage),
    DrainEfbCopies(EfbDrainRequest),
    Shutdown,
}

#[cfg(not(target_arch = "wasm32"))]
impl RenderWorker {
    fn run(mut self, work_rx: crossbeam_channel::Receiver<WorkerCommand>) {
        while let Ok(command) = work_rx.recv() {
            match command {
                WorkerCommand::Action(message) => self.exec(message),
                WorkerCommand::DrainEfbCopies(request) => self.drain_efb_copies(request),
                WorkerCommand::Shutdown => break,
            }
        }

        self.gx.submit_pending(&self.queue);
        #[cfg(feature = "renderdoc-capture")]
        if let Ok(mut rd) = self.renderdoc.lock() {
            rd.end_emulated_frame();
        }
        match self.gx.save_shader_cache() {
            Ok(n) => tracing::info!(num_variants = n, "saved shader cache"),
            Err(err) => tracing::warn!(?err, "failed to save shader cache"),
        }
        match self.gx.save_pipeline_cache() {
            Ok(n) => tracing::info!(num_pipelines = n, "saved pipeline cache"),
            Err(err) => tracing::warn!(?err, "failed to save pipeline cache"),
        }
    }

    fn exec(&mut self, message: ActionMessage) {
        if !message.vertices.is_empty() {
            self.gx.scratch_vertices.extend_from_slice(&message.vertices);
        }

        let resets_scratch = action_resets_vertex_scratch(&message.action);
        self.gx.process_action(&self.device, &self.queue, &message.action);

        match message.action {
            GxAction::PresentXfb { .. } => {
                #[cfg(feature = "renderdoc-capture")]
                if let Ok(mut rd) = self.renderdoc.lock() {
                    rd.end_emulated_frame();
                    rd.begin_emulated_frame();
                }

                *self.shared.output.lock().unwrap() = self.gx.xfb_texture.clone();
                if let Some(cb) = self.frame_ready_cb.get() {
                    cb(Instant::now());
                }
            }
            GxAction::Draw(boxed) => {
                let _ = self.recycled_draw_data_tx.send(boxed);
            }
            _ => {}
        }

        if resets_scratch {
            self.gx.scratch_vertices.clear();
        }
    }

    fn drain_efb_copies(&mut self, request: EfbDrainRequest) {
        let mut ram = unsafe {
            // The emu thread blocks on `done_tx` and holds the only mutable
            // RamViewMut while this command runs. FIFO channel ordering also
            // ensures all prior EFB copy commands have reached the worker.
            let mem1 = std::slice::from_raw_parts_mut(request.mem1_addr as *mut u8, request.mem1_len);
            let mem2 = std::slice::from_raw_parts_mut(request.mem2_addr as *mut u8, request.mem2_len);
            gecko::mmio::RamViewMut { mem1, mem2 }
        };
        self.gx.drain_pending_writebacks(&self.device, &self.queue, &mut ram);
        let _ = request.done_tx.send(());
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ThreadedSink {
    fn pending_vertices(&mut self) -> Vec<DrawVertex> {
        if self.scratch.len() <= self.scratch_sent_len {
            return Vec::new();
        }

        let vertices = self.scratch[self.scratch_sent_len..].to_vec();
        self.scratch_sent_len = self.scratch.len();
        vertices
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl RenderSink for ThreadedSink {
    fn exec(&mut self, action: GxAction) {
        let resets_scratch = action_resets_vertex_scratch(&action);
        let message = ActionMessage {
            action,
            vertices: self.pending_vertices(),
        };

        self.work_tx
            .send(WorkerCommand::Action(message))
            .expect("render worker thread stopped");

        if resets_scratch {
            self.scratch.clear();
            self.scratch_sent_len = 0;
        }
    }

    fn flush_efb_copies(&mut self, ram: &mut gecko::mmio::RamViewMut<'_>) {
        let (done_tx, done_rx) = crossbeam_channel::bounded(0);
        let request = EfbDrainRequest {
            mem1_addr: ram.mem1.as_mut_ptr() as usize,
            mem1_len: ram.mem1.len(),
            mem2_addr: ram.mem2.as_mut_ptr() as usize,
            mem2_len: ram.mem2.len(),
            done_tx,
        };

        self.work_tx
            .send(WorkerCommand::DrainEfbCopies(request))
            .expect("render worker thread stopped");
        done_rx.recv().expect("render worker thread stopped");
    }

    fn vertex_scratch(&mut self) -> &mut Vec<DrawVertex> {
        &mut self.scratch
    }

    fn take_draw_data(&mut self) -> Box<DrawData> {
        self.recycled_draw_data_rx.try_recv().unwrap_or_default()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for ThreadedSink {
    fn drop(&mut self) {
        if let Some(worker_thread) = self.worker_thread.take() {
            let _ = self.work_tx.send(WorkerCommand::Shutdown);
            if let Err(err) = worker_thread.join() {
                tracing::error!(?err, "render worker thread panicked");
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct InlineSink {
    gx: Arc<Mutex<GxRenderer>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    scratch: Vec<DrawVertex>,
    recycled_draw_data: Vec<Box<DrawData>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl InlineSink {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface_format: wgpu::TextureFormat,
    ) -> (Arc<Mutex<GxRenderer>>, Self) {
        let gx = Arc::new(Mutex::new(GxRenderer::new(&device, &queue, surface_format)));
        let sink = InlineSink {
            gx: gx.clone(),
            device,
            queue,
            scratch: Vec::new(),
            recycled_draw_data: Vec::new(),
        };
        (gx, sink)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl RenderSink for InlineSink {
    fn exec(&mut self, action: GxAction) {
        self.gx.lock().unwrap().process_action_with_external_scratch(
            &self.device,
            &self.queue,
            &action,
            &mut self.scratch,
        );

        if let GxAction::Draw(boxed) = action {
            self.recycled_draw_data.push(boxed);
        }
    }

    fn vertex_scratch(&mut self) -> &mut Vec<DrawVertex> {
        &mut self.scratch
    }

    fn flush_efb_copies(&mut self, ram: &mut gecko::mmio::RamViewMut<'_>) {
        self.gx
            .lock()
            .unwrap()
            .drain_pending_writebacks(&self.device, &self.queue, ram);
    }

    fn take_draw_data(&mut self) -> Box<DrawData> {
        self.recycled_draw_data.pop().unwrap_or_default()
    }
}

pub fn action_resets_vertex_scratch(action: &GxAction) -> bool {
    match action {
        GxAction::InvalidateCaches
        | GxAction::CopyXfb { .. }
        | GxAction::PresentXfb { .. }
        | GxAction::CopyEfbToTexture { .. } => true,
        #[cfg(not(target_arch = "wasm32"))]
        GxAction::DumpTextures { .. } => true,
        _ => false,
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct Renderer {
    shared: Arc<Shared>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    blit_sampler: wgpu::Sampler,
    target_aspect: TargetAspect,
    frame_ready_cb: Arc<OnceLock<FrameReadyCallback>>,
    #[cfg(feature = "renderdoc-capture")]
    renderdoc: Arc<Mutex<crate::renderdoc_capture::RenderDocCapture>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Renderer {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        target_aspect: TargetAspect,
    ) -> (Self, ThreadedSink) {
        let mut gx = GxRenderer::new(&device, &queue, surface_format);
        gx.prewarm_pipeline_cache(&device);

        // Initial shared output: the XFB texture (black until first PresentXfb).
        let shared = Arc::new(Shared {
            output: Mutex::new(gx.xfb_texture.clone()),
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
            bind_group_layouts: &[Some(&blit_bind_group_layout)],
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

        #[cfg(feature = "renderdoc-capture")]
        let renderdoc = Arc::new(Mutex::new(crate::renderdoc_capture::RenderDocCapture::new()));

        let (work_tx, work_rx) = crossbeam_channel::bounded(WORK_QUEUE_LIMIT);
        let (recycled_draw_data_tx, recycled_draw_data_rx) = crossbeam_channel::unbounded();
        let worker = RenderWorker {
            gx,
            device: device.clone(),
            queue: queue.clone(),
            shared: shared.clone(),
            frame_ready_cb: frame_ready_cb.clone(),
            recycled_draw_data_tx,
            #[cfg(feature = "renderdoc-capture")]
            renderdoc: renderdoc.clone(),
        };
        let worker_thread = std::thread::Builder::new()
            .name("backend-wgpu render".to_string())
            .spawn(move || worker.run(work_rx))
            .expect("failed to spawn render worker thread");

        let sink = ThreadedSink {
            work_tx,
            recycled_draw_data_rx,
            worker_thread: Some(worker_thread),
            scratch: Vec::new(),
            scratch_sent_len: 0,
        };

        let renderer = Renderer {
            shared,
            device,
            queue,
            blit_pipeline,
            blit_bind_group_layout,
            blit_sampler,
            target_aspect,
            frame_ready_cb,
            #[cfg(feature = "renderdoc-capture")]
            renderdoc,
        };

        (renderer, sink)
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

    /// Read back the most recently presented XFB frame at native emulated
    /// resolution. Blocks until the GPU copy completes.
    pub fn capture_xfb(&self) -> Option<crate::capture::CapturedFrame> {
        let texture = self.shared.output.lock().unwrap().clone();
        crate::capture::capture_texture(&self.device, &self.queue, &texture)
    }

    /// Blit the latest XFB output to the given render target. `target_size`
    /// is the destination view's pixel size; used to letterbox/pillarbox the
    /// XFB to `self.target_aspect`. Called by the windowing thread on each
    /// redraw.
    pub fn blit(&self, queue: &wgpu::Queue, target: &wgpu::TextureView, target_size: (u32, u32)) {
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("xfb_blit_encoder"),
        });
        self.blit_into_encoder(
            &mut encoder,
            target,
            target_size,
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
        );
        queue.submit([encoder.finish()]);
    }

    /// `blit` variant that writes into a caller-owned encoder, so the blit can
    /// land inside someone else's frame command buffer (e.g. iced's shader
    /// widget) instead of being submitted on its own.
    pub fn blit_into_encoder(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        target_size: (u32, u32),
        load: wgpu::LoadOp<wgpu::Color>,
    ) {
        let output = self.shared.output.lock().unwrap().clone();
        let view = output.create_view(&Default::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit_bg"),
            layout: &self.blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.blit_sampler),
                },
            ],
        });

        encoder.push_debug_group("XFB Blit To Surface");
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xfb_blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
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
