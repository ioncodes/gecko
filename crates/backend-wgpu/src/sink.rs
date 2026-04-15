use crate::GxRenderer;
use crossbeam_channel::{Receiver, Sender, bounded};
#[cfg(feature = "efb-writeback")]
use gecko::host::EfbWriteback;
use gecko::host::{GxAction, RenderSink};
use std::sync::{Arc, Mutex};

const CHANNEL_CAPACITY: usize = 8192;

/// Holds the XFB output texture view that the worker updates and the main
/// thread reads for blitting.
pub struct Shared {
    pub output: Mutex<wgpu::TextureView>,
}

fn worker(mut gx: GxRenderer, device: wgpu::Device, queue: wgpu::Queue, shared: Arc<Shared>, rx: Receiver<GxAction>) {
    while let Ok(action) = rx.recv() {
        gx.process_action(&device, &queue, &action);

        // After a PresentXfb, update the shared output so the main thread
        // picks up the latest composited frame on its next blit.
        if matches!(action, GxAction::PresentXfb { .. }) {
            let mut output = shared.output.lock().unwrap();
            *output = gx.xfb_view.clone();
        }
    }
}

#[derive(Clone)]
pub struct Renderer {
    tx: Sender<GxAction>,
    shared: Arc<Shared>,
    device: wgpu::Device,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    blit_sampler: wgpu::Sampler,
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
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, surface_format: wgpu::TextureFormat) -> Self {
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

    /// Blit the latest XFB output to the given render target. Called by the
    /// main thread on each redraw.
    pub fn blit(&self, queue: &wgpu::Queue, target: &wgpu::TextureView) {
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
        let _ = self.tx.send(action);
    }
}
