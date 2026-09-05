use gecko::host::GxAction;

#[cfg(not(target_arch = "wasm32"))]
use crate::GxRenderer;
#[cfg(not(target_arch = "wasm32"))]
use gecko::host::{DrawData, DrawSegment, DrawState, DrawVertex, RenderSink};
#[cfg(all(not(target_arch = "wasm32"), feature = "gx-stats"))]
use std::sync::atomic::Ordering;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(not(target_arch = "wasm32"))]
pub type FrameReadyCallback = Box<dyn Fn(Instant) + Send + Sync>;

#[cfg(not(target_arch = "wasm32"))]
// Keep only a few GX batches in flight. This used to count individual actions,
// so leaving it at 4096 after batching would allow thousands of whole frames to
// accumulate behind the render worker.
const WORK_QUEUE_LIMIT: usize = 8;

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
    recycled_batches_rx: crossbeam_channel::Receiver<ActionBatch>,
    #[cfg(feature = "gx-stats")]
    stats: Arc<crate::RendererStats>,
    worker_thread: Option<JoinHandle<()>>,
    pending_actions: Vec<GxAction>,
    scratch: Vec<DrawVertex>,
    immediate_draw_data: Option<Box<DrawData>>,
    efb_writeback_pending: bool,
    pending_efb_textures: Vec<PendingEfbTexture>,
}

#[cfg(not(target_arch = "wasm32"))]
struct PendingEfbTexture {
    addr: u32,
    end: u32,
    width: u32,
    height: u32,
    format: gecko::flipper::gx::draw::TextureFormat,
}

#[cfg(not(target_arch = "wasm32"))]
struct RenderWorker {
    gx: GxRenderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    shared: Arc<Shared>,
    frame_ready_cb: Arc<OnceLock<FrameReadyCallback>>,
    recycled_draw_data_tx: crossbeam_channel::Sender<Box<DrawData>>,
    recycled_batches_tx: crossbeam_channel::Sender<ActionBatch>,
    #[cfg(feature = "renderdoc-capture")]
    renderdoc: Arc<Mutex<crate::renderdoc_capture::RenderDocCapture>>,
}

#[cfg(not(target_arch = "wasm32"))]
struct ActionBatch {
    actions: Vec<GxAction>,
    vertices: Vec<DrawVertex>,
    resets_scratch: bool,
}

#[cfg(not(target_arch = "wasm32"))]
struct EfbDrainRequest {
    mem1_addr: usize,
    mem1_len: usize,
    mem2_addr: usize,
    mem2_len: usize,
    batch: ActionBatch,
    done_tx: crossbeam_channel::Sender<ActionBatch>,
}

#[cfg(not(target_arch = "wasm32"))]
enum WorkerCommand {
    Actions(ActionBatch),
    DrainEfbCopies(EfbDrainRequest),
    PeekEfbDepth {
        x: u32,
        y: u32,
        batch: ActionBatch,
        done_tx: crossbeam_channel::Sender<(ActionBatch, u32)>,
    },
    Shutdown,
}

#[cfg(not(target_arch = "wasm32"))]
impl RenderWorker {
    fn run(mut self, work_rx: crossbeam_channel::Receiver<WorkerCommand>) {
        while let Ok(command) = work_rx.recv() {
            match command {
                WorkerCommand::Actions(batch) => self.exec_batch(batch),
                WorkerCommand::DrainEfbCopies(request) => self.drain_efb_copies(request),
                WorkerCommand::PeekEfbDepth {
                    x,
                    y,
                    mut batch,
                    done_tx,
                } => {
                    let worker_scratch = self.gx.replace_vertex_scratch(std::mem::take(&mut batch.vertices));
                    for action in batch.actions.drain(..) {
                        self.exec_action(action);
                    }

                    let depth = self.gx.peek_efb_depth(&self.device, &self.queue, x, y);
                    batch.vertices = self.gx.replace_vertex_scratch(worker_scratch);

                    let _ = done_tx.send((batch, depth));
                }
                WorkerCommand::Shutdown => break,
            }
        }

        let _ = self.gx.submit_pending(&self.queue);
        self.gx.poll_compiled_pipelines();
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

    fn exec_action(&mut self, action: GxAction) {
        self.gx.process_action(&self.device, &self.queue, &action);

        if matches!(&action, GxAction::CopyEfbToTexture { .. }) {
            let _ = self.gx.submit_pending(&self.queue);
        }

        match action {
            GxAction::PresentXfb { .. } | GxAction::PresentRawXfb { .. } => {
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
    }

    fn exec_batch_returning_vertices(&mut self, batch: &mut ActionBatch) {
        let worker_scratch = self.gx.replace_vertex_scratch(std::mem::take(&mut batch.vertices));
        for action in batch.actions.drain(..) {
            self.exec_action(action);
        }
        batch.vertices = self.gx.replace_vertex_scratch(worker_scratch);
    }

    fn exec_batch(&mut self, mut batch: ActionBatch) {
        #[cfg(feature = "gx-stats")]
        let batch_started = std::time::Instant::now();

        if batch.resets_scratch {
            self.exec_batch_returning_vertices(&mut batch);
            batch.vertices.clear();

            debug_assert!(batch.actions.is_empty());
            debug_assert!(batch.vertices.is_empty());

            let _ = self.recycled_batches_tx.send(batch);
        } else {
            let _old_scratch = self.gx.replace_vertex_scratch(batch.vertices);
            for action in batch.actions {
                self.exec_action(action);
            }
        }

        #[cfg(feature = "gx-stats")]
        self.gx
            .stats
            .worker_batch_cpu_ns
            .fetch_add(batch_started.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    fn drain_efb_copies(&mut self, mut request: EfbDrainRequest) {
        #[cfg(feature = "gx-stats")]
        let batch_started = std::time::Instant::now();

        let expected_vertex_len = request.batch.vertices.len();
        self.exec_batch_returning_vertices(&mut request.batch);
        debug_assert_eq!(request.batch.vertices.len(), expected_vertex_len);

        let mut ram = unsafe {
            // The emu thread blocks on `done_tx` and holds the only mutable
            // RamViewMut while this command runs. FIFO channel ordering also
            // ensures all prior EFB copy commands have reached the worker.
            let mem1 = std::slice::from_raw_parts_mut(request.mem1_addr as *mut u8, request.mem1_len);
            let mem2 = std::slice::from_raw_parts_mut(request.mem2_addr as *mut u8, request.mem2_len);
            gecko::mmio::RamViewMut {
                mem1,
                mem2,
                memory_page_generations: &[],
            }
        };
        #[cfg(feature = "gx-stats")]
        let writeback_count = self.gx.pending_writebacks.len() as u64;
        #[cfg(feature = "gx-stats")]
        let writeback_started = std::time::Instant::now();
        self.gx.drain_pending_writebacks(&self.device, &self.queue, &mut ram);

        #[cfg(feature = "gx-stats")]
        {
            self.gx.stats.efb_drain_requests.fetch_add(1, Ordering::Relaxed);
            if writeback_count != 0 {
                self.gx.stats.efb_drain_nonempty.fetch_add(1, Ordering::Relaxed);
                self.gx
                    .stats
                    .efb_writebacks
                    .fetch_add(writeback_count, Ordering::Relaxed);
                self.gx
                    .stats
                    .efb_writeback_cpu_ns
                    .fetch_add(writeback_started.elapsed().as_nanos() as u64, Ordering::Relaxed);
            }
            self.gx
                .stats
                .worker_batch_cpu_ns
                .fetch_add(batch_started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }

        let _ = request.done_tx.send(request.batch);
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ThreadedSink {
    fn reclaim_batch(&mut self) {
        if !self.pending_actions.is_empty() || !self.scratch.is_empty() {
            return;
        }

        if let Ok(batch) = self.recycled_batches_rx.try_recv() {
            debug_assert!(batch.actions.is_empty());
            debug_assert!(batch.vertices.is_empty());
            self.pending_actions = batch.actions;
            self.scratch = batch.vertices;
        }
    }

    fn take_batch(&mut self, resets_scratch: bool) -> ActionBatch {
        ActionBatch {
            actions: std::mem::take(&mut self.pending_actions),
            vertices: std::mem::take(&mut self.scratch),
            resets_scratch,
        }
    }

    fn send_reset_batch(&mut self) {
        let batch = self.take_batch(true);
        #[cfg(feature = "gx-stats")]
        let send_started = std::time::Instant::now();
        self.work_tx
            .send(WorkerCommand::Actions(batch))
            .expect("render worker thread stopped");
        #[cfg(feature = "gx-stats")]
        {
            self.stats.batches_sent.fetch_add(1, Ordering::Relaxed);
            self.stats
                .channel_high_water
                .fetch_max(self.work_tx.len() as u64, Ordering::Relaxed);
            self.stats
                .queue_wait_ns
                .fetch_add(send_started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        self.reclaim_batch();
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl RenderSink for ThreadedSink {
    fn peek_efb_depth(&mut self, x: u32, y: u32) -> u32 {
        let (done_tx, done_rx) = crossbeam_channel::bounded(0);
        let command = WorkerCommand::PeekEfbDepth {
            x,
            y,
            batch: self.take_batch(false),
            done_tx,
        };
        self.work_tx.send(command).expect("render worker thread stopped");
        let (batch, depth) = done_rx.recv().expect("render worker thread stopped");
        self.pending_actions = batch.actions;
        self.scratch = batch.vertices;

        depth
    }

    fn exec(&mut self, action: GxAction) {
        #[cfg(feature = "gx-stats")]
        self.stats.actions_sent.fetch_add(1, Ordering::Relaxed);

        let action = match action {
            GxAction::Draw(mut draw) if draw.state.is_none() => {
                if let Some(GxAction::Draw(previous)) = self.pending_actions.last_mut() {
                    previous.segments.append(&mut draw.segments);
                    self.immediate_draw_data = Some(draw);

                    return;
                }

                GxAction::Draw(draw)
            }
            action => action,
        };

        if let GxAction::CopyEfbToTexture {
            dest_addr,
            src_x,
            src_y,
            src_w,
            src_h,
            copy_format,
            mipmap,
            stride,
            depth_copy,
            ..
        } = &action
        {
            use gecko::flipper::gx::texture::{CopyFormat, encoded_row_bytes, encoded_row_count};
            self.efb_writeback_pending = true;
            let format = if *depth_copy {
                CopyFormat::from_u8_depth(*copy_format)
            } else {
                CopyFormat::from_u8_color(*copy_format)
            };

            if let Some(format) = format {
                let divisor = if *mipmap { 2 } else { 1 };
                let width = src_w.min(&crate::EFB_WIDTH.saturating_sub(*src_x)) / divisor;
                let height = src_h.min(&crate::EFB_HEIGHT.saturating_sub(*src_y)) / divisor;
                let row_bytes = encoded_row_bytes(width, format);
                let rows = encoded_row_count(height, format);
                let len = rows.saturating_sub(1) * *stride as usize + row_bytes;
                let end = dest_addr.saturating_add(len as u32);
                self.pending_efb_textures
                    .retain(|copy| copy.end <= *dest_addr || end <= copy.addr);
                if width > 0 && height > 0 && *stride as usize == row_bytes {
                    self.pending_efb_textures.push(PendingEfbTexture {
                        addr: *dest_addr,
                        end,
                        width,
                        height,
                        format: format.base_texture_format(),
                    });
                }
            }
        } else if matches!(&action, GxAction::InvalidateCaches) {
            self.pending_efb_textures.clear();
        }

        let resets_scratch = action_resets_vertex_scratch(&action);
        self.pending_actions.push(action);

        if resets_scratch {
            self.send_reset_batch();
        }
    }

    fn exec_draw(&mut self, segment: DrawSegment, state: Option<DrawState>) {
        self.exec_draw_batch(std::slice::from_ref(&segment), state);
    }

    fn exec_draw_batch(&mut self, segments: &[DrawSegment], state: Option<DrawState>) {
        if segments.is_empty() {
            return;
        }

        #[cfg(feature = "gx-stats")]
        self.stats
            .actions_sent
            .fetch_add(segments.len() as u64, Ordering::Relaxed);

        self.reclaim_batch();

        if state.is_none()
            && let Some(GxAction::Draw(previous)) = self.pending_actions.last_mut()
        {
            previous.segments.extend_from_slice(segments);

            return;
        }

        let mut draw = self.take_draw_data();
        draw.segments.clear();
        draw.segments.extend_from_slice(segments);
        draw.state = state;
        self.pending_actions.push(GxAction::Draw(draw));
    }

    fn has_pending_efb_texture(
        &self,
        addr: u32,
        width: u32,
        height: u32,
        fmt: gecko::flipper::gx::draw::TextureFormat,
    ) -> bool {
        self.pending_efb_textures
            .iter()
            .any(|copy| copy.addr == addr && copy.width == width && copy.height == height && copy.format == fmt)
    }

    fn flush_efb_copies(&mut self, ram: &mut gecko::mmio::RamViewMut<'_>) {
        if !self.efb_writeback_pending {
            return;
        }

        let (done_tx, done_rx) = crossbeam_channel::bounded(0);
        let request = EfbDrainRequest {
            mem1_addr: ram.mem1.as_mut_ptr() as usize,
            mem1_len: ram.mem1.len(),
            mem2_addr: ram.mem2.as_mut_ptr() as usize,
            mem2_len: ram.mem2.len(),
            batch: self.take_batch(false),
            done_tx,
        };

        #[cfg(feature = "gx-stats")]
        let drain_started = std::time::Instant::now();
        self.work_tx
            .send(WorkerCommand::DrainEfbCopies(request))
            .expect("render worker thread stopped");
        let batch = done_rx.recv().expect("render worker thread stopped");
        #[cfg(feature = "gx-stats")]
        {
            self.stats.batches_sent.fetch_add(1, Ordering::Relaxed);
            self.stats
                .channel_high_water
                .fetch_max(self.work_tx.len() as u64, Ordering::Relaxed);
            self.stats
                .efb_drain_wait_ns
                .fetch_add(drain_started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        debug_assert!(batch.actions.is_empty());
        self.pending_actions = batch.actions;
        self.scratch = batch.vertices;
        self.efb_writeback_pending = false;
        self.pending_efb_textures.clear();
    }

    fn vertex_scratch(&mut self) -> &mut Vec<DrawVertex> {
        self.reclaim_batch();
        &mut self.scratch
    }

    fn take_draw_data(&mut self) -> Box<DrawData> {
        self.reclaim_batch();
        self.immediate_draw_data
            .take()
            .or_else(|| self.recycled_draw_data_rx.try_recv().ok())
            .unwrap_or_default()
    }

    fn render_stats(&self) -> gecko::host::RenderStats {
        #[cfg(feature = "gx-stats")]
        {
            return self
                .stats
                .snapshot(self.work_tx.len(), self.work_tx.capacity().unwrap_or(0));
        }

        #[cfg(not(feature = "gx-stats"))]
        gecko::host::RenderStats::default()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for ThreadedSink {
    fn drop(&mut self) {
        if let Some(worker_thread) = self.worker_thread.take() {
            if !self.pending_actions.is_empty() || !self.scratch.is_empty() {
                let batch = self.take_batch(false);
                let _ = self.work_tx.send(WorkerCommand::Actions(batch));
                #[cfg(feature = "gx-stats")]
                self.stats.batches_sent.fetch_add(1, Ordering::Relaxed);
            }
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
        let gx = Arc::new(Mutex::new(GxRenderer::new(&device, &queue, surface_format, 1)));
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
    fn peek_efb_depth(&mut self, x: u32, y: u32) -> u32 {
        let mut gx = self.gx.lock().unwrap();
        let depth = gx.peek_efb_depth(&self.device, &self.queue, x, y);
        self.scratch.truncate(gx.scratch_vertices.len());

        depth
    }

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

    fn reset_efb(&mut self) {
        self.gx.lock().unwrap().reset_efb(&self.device, &self.queue);
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
        | GxAction::PresentRawXfb { .. }
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
        efb_scale: u32,
    ) -> (Self, ThreadedSink) {
        let mut gx = GxRenderer::new(&device, &queue, surface_format, efb_scale);
        gx.prewarm_pipeline_cache(&device);
        #[cfg(feature = "gx-stats")]
        let stats = gx.stats.clone();

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
        let (recycled_batches_tx, recycled_batches_rx) = crossbeam_channel::unbounded();
        let worker = RenderWorker {
            gx,
            device: device.clone(),
            queue: queue.clone(),
            shared: shared.clone(),
            frame_ready_cb: frame_ready_cb.clone(),
            recycled_draw_data_tx,
            recycled_batches_tx,
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
            recycled_batches_rx,
            #[cfg(feature = "gx-stats")]
            stats,
            worker_thread: Some(worker_thread),
            pending_actions: Vec::new(),
            scratch: Vec::new(),
            immediate_draw_data: None,
            efb_writeback_pending: false,
            pending_efb_textures: Vec::new(),
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

    /// Read back the most recently presented XFB frame at the emulated
    /// resolution times the EFB scale. Blocks until the GPU copy completes.
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
