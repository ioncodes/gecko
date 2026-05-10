use crate::GxRenderer;
use crossbeam_channel::{Receiver, Sender, bounded};
#[cfg(feature = "efb-writeback")]
use gecko::host::EfbWriteback;
use gecko::host::{DrawData, GxAction, RenderSink};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(feature = "renderdoc-capture")]
use std::time::Duration;

const CHANNEL_CAPACITY: usize = 65536;
const RECYCLE_CAPACITY: usize = 8192;
const BATCH_SIZE: usize = 256;
const BATCH_RECYCLE_CAPACITY: usize = 64;

/// Holds the XFB output texture view that the worker updates and the main
/// thread reads for blitting.
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

enum WorkerMsg {
    ActionBatch(Vec<GxAction>),
    #[cfg(feature = "renderdoc-capture")]
    BeginEmulatedFrame {
        ack: Sender<()>,
    },
    #[cfg(feature = "renderdoc-capture")]
    EndEmulatedFrame,
    #[cfg(feature = "renderdoc-capture")]
    CaptureNextEmulatedFrame,
    #[cfg(feature = "renderdoc-capture")]
    StartFrameCapture,
    #[cfg(feature = "renderdoc-capture")]
    EndFrameCapture,
    #[cfg(feature = "renderdoc-capture")]
    TriggerCapture,
}

struct Recyclers {
    boxes: Sender<Box<DrawData>>,
    batches: Sender<Vec<GxAction>>,
}

fn worker(
    mut gx: GxRenderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    shared: Arc<Shared>,
    rx: Receiver<WorkerMsg>,
    recyclers: Recyclers,
) {
    #[cfg(feature = "renderdoc-capture")]
    let mut renderdoc = crate::renderdoc_capture::RenderDocCapture::new();

    while let Ok(msg) = rx.recv() {
        let mut batch = match msg {
            WorkerMsg::ActionBatch(batch) => batch,
            #[cfg(feature = "renderdoc-capture")]
            WorkerMsg::BeginEmulatedFrame { ack } => {
                renderdoc.begin_emulated_frame();
                submit_debug_marker(&device, &queue, "Emulated Frame Begin", "GX FIFO execution begins");
                let _ = ack.send(());
                continue;
            }
            #[cfg(feature = "renderdoc-capture")]
            WorkerMsg::EndEmulatedFrame => {
                submit_debug_marker(&device, &queue, "Emulated Frame End", "GX FIFO execution ends");
                renderdoc.end_emulated_frame();
                continue;
            }
            #[cfg(feature = "renderdoc-capture")]
            WorkerMsg::CaptureNextEmulatedFrame => {
                renderdoc.request_next_emulated_frame();
                continue;
            }
            #[cfg(feature = "renderdoc-capture")]
            WorkerMsg::StartFrameCapture => {
                renderdoc.start_frame_capture();
                continue;
            }
            #[cfg(feature = "renderdoc-capture")]
            WorkerMsg::EndFrameCapture => {
                renderdoc.end_frame_capture();
                continue;
            }
            #[cfg(feature = "renderdoc-capture")]
            WorkerMsg::TriggerCapture => {
                renderdoc.trigger_capture();
                continue;
            }
        };

        for action in batch.drain(..) {
            gx.process_action(&device, &queue, &action);

            match action {
                GxAction::PresentXfb { .. } => {
                    let mut output = shared.output.lock().unwrap();
                    *output = gx.xfb_view.clone();
                }
                GxAction::Draw(mut boxed) => {
                    boxed.vertices.clear();
                    let _ = recyclers.boxes.try_send(boxed);
                }
                _ => {}
            }
        }

        // Hand the drained Vec back to the producer for capacity reuse.
        // On full or closed channel, drop and the producer will allocate
        // a fresh batch on its next flush.
        let _ = recyclers.batches.try_send(batch);
    }
}

#[cfg(feature = "renderdoc-capture")]
fn submit_debug_marker(device: &wgpu::Device, queue: &wgpu::Queue, group: &'static str, marker: &'static str) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(group) });
    encoder.push_debug_group(group);
    encoder.insert_debug_marker(marker);
    encoder.pop_debug_group();
    queue.submit([encoder.finish()]);
}

#[derive(Clone)]
pub struct Renderer {
    tx: Sender<WorkerMsg>,
    shared: Arc<Shared>,
    device: wgpu::Device,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    blit_sampler: wgpu::Sampler,
    target_aspect: TargetAspect,
    actions_sent: Arc<AtomicU64>,
    /// Receiver end of the EFB-to-texture writeback channel. Taken by the
    /// emulator setup code (via [`Renderer::take_writeback_rx`]) and
    /// installed into `GraphicsProcessor::efb_writeback_rx`. Wrapped in
    /// `Arc<Mutex<Option<_>>>` so `Renderer` stays `Clone`. Only built when
    /// `efb-writeback` is enabled.
    #[cfg(feature = "efb-writeback")]
    writeback_rx: Arc<Mutex<Option<Receiver<EfbWriteback>>>>,
    recycle_rx: Arc<Mutex<Option<Receiver<Box<DrawData>>>>>,
    batch_recycle_rx: Arc<Mutex<Option<Receiver<Vec<GxAction>>>>>,
}

impl Renderer {
    /// Create the renderer, spawning the worker thread. The caller must
    /// provide a wgpu device and queue.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        target_aspect: TargetAspect,
    ) -> Self {
        #[cfg(feature = "efb-writeback")]
        let mut gx = GxRenderer::new(&device, &queue, surface_format);
        #[cfg(not(feature = "efb-writeback"))]
        let gx = GxRenderer::new(&device, &queue, surface_format);

        // Writeback channel: GxRenderer sends encoded EFB-to-texture bytes,
        // GraphicsProcessor consumes them synchronously on the emu thread.
        // Only created with `efb-writeback`.
        #[cfg(feature = "efb-writeback")]
        let writeback_rx = {
            let (writeback_tx, writeback_rx) = bounded::<EfbWriteback>(CHANNEL_CAPACITY);
            gx.set_efb_writeback_tx(writeback_tx);
            writeback_rx
        };

        // Initial shared output: the XFB view (black until first PresentXfb).
        let shared = Arc::new(Shared {
            output: Mutex::new(gx.xfb_view.clone()),
        });

        // Build the blit pipeline on the main-thread side.
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

        let (tx, rx) = bounded(CHANNEL_CAPACITY);
        let (boxes_tx, recycle_rx) = bounded::<Box<DrawData>>(RECYCLE_CAPACITY);
        let (batches_tx, batch_recycle_rx) = bounded::<Vec<GxAction>>(BATCH_RECYCLE_CAPACITY);
        let recyclers = Recyclers {
            boxes: boxes_tx,
            batches: batches_tx,
        };

        // Spawn the worker.
        let worker_shared = shared.clone();
        let worker_device = device.clone();
        let worker_queue = queue.clone();
        std::thread::Builder::new()
            .name("gx-renderer".into())
            .spawn(move || worker(gx, worker_device, worker_queue, worker_shared, rx, recyclers))
            .expect("failed to spawn renderer worker");

        Renderer {
            tx,
            shared,
            device,
            blit_pipeline,
            blit_bind_group_layout,
            blit_sampler,
            target_aspect,
            actions_sent: Arc::new(AtomicU64::new(0)),
            #[cfg(feature = "efb-writeback")]
            writeback_rx: Arc::new(Mutex::new(Some(writeback_rx))),
            recycle_rx: Arc::new(Mutex::new(Some(recycle_rx))),
            batch_recycle_rx: Arc::new(Mutex::new(Some(batch_recycle_rx))),
        }
    }

    /// Take the writeback receiver once. Returns `Some` on the first call,
    /// `None` thereafter. The caller installs it into
    /// `GraphicsProcessor::efb_writeback_rx`. Only available when the
    /// `efb-writeback` feature is enabled.
    #[cfg(feature = "efb-writeback")]
    pub fn take_writeback_rx(&self) -> Option<Receiver<EfbWriteback>> {
        self.writeback_rx.lock().ok()?.take()
    }

    pub fn take_recycle_rx(&self) -> Option<Receiver<Box<DrawData>>> {
        self.recycle_rx.lock().ok()?.take()
    }

    pub fn target_aspect(&self) -> TargetAspect {
        self.target_aspect
    }

    #[cfg(feature = "renderdoc-capture")]
    pub fn begin_renderdoc_emulated_frame(&self) {
        let (ack_tx, ack_rx) = bounded(0);
        if self.tx.send(WorkerMsg::BeginEmulatedFrame { ack: ack_tx }).is_err() {
            tracing::warn!("failed to send RenderDoc frame-begin marker to renderer worker");
            return;
        }

        if ack_rx.recv_timeout(Duration::from_secs(1)).is_err() {
            tracing::warn!("timed out waiting for renderer worker to begin RenderDoc frame");
        }
    }

    #[cfg(feature = "renderdoc-capture")]
    pub fn end_renderdoc_emulated_frame(&self) {
        let _ = self.tx.send(WorkerMsg::EndEmulatedFrame);
    }

    #[cfg(feature = "renderdoc-capture")]
    pub fn capture_next_renderdoc_emulated_frame(&self) {
        let _ = self.tx.send(WorkerMsg::CaptureNextEmulatedFrame);
    }

    #[cfg(feature = "renderdoc-capture")]
    pub fn start_renderdoc_frame_capture(&self) {
        let _ = self.tx.send(WorkerMsg::StartFrameCapture);
    }

    #[cfg(feature = "renderdoc-capture")]
    pub fn end_renderdoc_frame_capture(&self) {
        let _ = self.tx.send(WorkerMsg::EndFrameCapture);
    }

    #[cfg(feature = "renderdoc-capture")]
    pub fn trigger_renderdoc_capture(&self) {
        let _ = self.tx.send(WorkerMsg::TriggerCapture);
    }

    /// Blit the latest XFB output to the given render target. `target_size`
    /// is the destination view's pixel size; used to letterbox/pillarbox the
    /// XFB to `self.target_aspect`. Called by the main thread on each redraw.
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

impl Renderer {
    /// Build a producer-side sink that batches `GxAction`s before sending
    /// them to the worker. The first call returns a sink with the
    /// recycler receiver installed; subsequent calls return a sink that
    /// allocates a fresh `Vec` on each flush. Mirrors the `take_*_rx`
    /// pattern so the renderer handle remains usable for blitting and
    /// renderdoc markers afterward.
    pub fn take_batching_sink(&self) -> BatchingSink {
        let batch_recycle_rx = self.batch_recycle_rx.lock().ok().and_then(|mut g| g.take());
        BatchingSink {
            tx: self.tx.clone(),
            actions_sent: self.actions_sent.clone(),
            channel_capacity: CHANNEL_CAPACITY,
            batch: Vec::with_capacity(BATCH_SIZE),
            batch_recycle_rx,
        }
    }
}

pub struct BatchingSink {
    tx: Sender<WorkerMsg>,
    actions_sent: Arc<AtomicU64>,
    channel_capacity: usize,
    batch: Vec<GxAction>,
    batch_recycle_rx: Option<Receiver<Vec<GxAction>>>,
}

impl BatchingSink {
    fn flush(&mut self) {
        if self.batch.is_empty() {
            return;
        }

        let next = self
            .batch_recycle_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
            .unwrap_or_else(|| Vec::with_capacity(BATCH_SIZE));
        let to_send = std::mem::replace(&mut self.batch, next);
        let len = to_send.len() as u64;

        if self.tx.send(WorkerMsg::ActionBatch(to_send)).is_ok() {
            self.actions_sent.fetch_add(len, Ordering::Relaxed);
        }
    }
}

impl Drop for BatchingSink {
    fn drop(&mut self) {
        self.flush();
    }
}

impl RenderSink for BatchingSink {
    fn exec(&mut self, action: GxAction) {
        let force_flush = matches!(action, GxAction::PresentXfb { .. } | GxAction::CopyEfbToTexture { .. });

        self.batch.push(action);

        if force_flush || self.batch.len() >= BATCH_SIZE {
            self.flush();
        }
    }

    fn actions_sent_total(&self) -> u64 {
        self.actions_sent.load(Ordering::Relaxed)
    }

    fn channel_len(&self) -> usize {
        self.tx.len()
    }

    fn channel_capacity(&self) -> Option<usize> {
        Some(self.channel_capacity)
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
