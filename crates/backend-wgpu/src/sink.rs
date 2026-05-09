use crate::GxRenderer;
use crossbeam_channel::{Receiver, Sender, bounded};
#[cfg(feature = "efb-writeback")]
use gecko::host::EfbWriteback;
use gecko::host::{GxAction, RenderSink};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(feature = "renderdoc-capture")]
use std::time::Duration;

const CHANNEL_CAPACITY: usize = 65536;

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
    Action(GxAction),
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

fn worker(mut gx: GxRenderer, device: wgpu::Device, queue: wgpu::Queue, shared: Arc<Shared>, rx: Receiver<WorkerMsg>) {
    #[cfg(feature = "renderdoc-capture")]
    let mut renderdoc = crate::renderdoc_capture::RenderDocCapture::new();

    while let Ok(msg) = rx.recv() {
        let action = match msg {
            WorkerMsg::Action(action) => action,
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

        gx.process_action(&device, &queue, &action);

        // After a PresentXfb, update the shared output so the main thread
        // picks up the latest composited frame on its next blit.
        if matches!(action, GxAction::PresentXfb { .. }) {
            let mut output = shared.output.lock().unwrap();
            *output = gx.xfb_view.clone();
        }
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

        // Spawn the worker.
        let worker_shared = shared.clone();
        let worker_device = device.clone();
        let worker_queue = queue.clone();
        std::thread::Builder::new()
            .name("gx-renderer".into())
            .spawn(move || worker(gx, worker_device, worker_queue, worker_shared, rx))
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

impl RenderSink for Renderer {
    fn exec(&mut self, action: GxAction) {
        self.actions_sent.fetch_add(1, Ordering::Relaxed);
        let _ = self.tx.send(WorkerMsg::Action(action));
    }

    fn actions_sent_total(&self) -> u64 {
        self.actions_sent.load(Ordering::Relaxed)
    }

    fn channel_len(&self) -> usize {
        self.tx.len()
    }

    fn channel_capacity(&self) -> Option<usize> {
        self.tx.capacity()
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
