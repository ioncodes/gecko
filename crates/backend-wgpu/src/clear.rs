#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ClearUniforms {
    color: [f32; 4],
    depth: f32,
    _pad: [f32; 3],
}

fn clear_mask_label(color_update: bool, alpha_update: bool, z_update: bool) -> &'static str {
    match (color_update, alpha_update, z_update) {
        (true, true, true) => "rgba+z",
        (true, false, true) => "rgb+z",
        (false, true, true) => "alpha+z",
        (false, false, true) => "z",
        (true, true, false) => "rgba",
        (true, false, false) => "rgb",
        (false, true, false) => "alpha",
        (false, false, false) => "none",
    }
}

pub(crate) struct EfbClear {
    pipeline_all_depth: wgpu::RenderPipeline,
    pipeline_rgb_depth: wgpu::RenderPipeline,
    pipeline_alpha_depth: wgpu::RenderPipeline,
    pipeline_none_depth: wgpu::RenderPipeline,
    pipeline_all: wgpu::RenderPipeline,
    pipeline_rgb: wgpu::RenderPipeline,
    pipeline_alpha: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

fn create_clear_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    color_format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    sample_count: u32,
    color_write_mask: wgpu::ColorWrites,
    depth_write_enabled: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("efb_clear_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                blend: None,
                write_mask: color_write_mask,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled,
            depth_compare: wgpu::CompareFunction::Always,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview_mask: None,
        cache: None,
    })
}

impl EfbClear {
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("efb_clear_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/clear.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("efb_clear_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("efb_clear_layout"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });

        let rgb = wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN | wgpu::ColorWrites::BLUE;
        let pipeline_all_depth = create_clear_pipeline(
            device,
            &shader,
            &layout,
            color_format,
            depth_format,
            sample_count,
            wgpu::ColorWrites::ALL,
            true,
        );
        let pipeline_rgb_depth = create_clear_pipeline(
            device,
            &shader,
            &layout,
            color_format,
            depth_format,
            sample_count,
            rgb,
            true,
        );
        let pipeline_alpha_depth = create_clear_pipeline(
            device,
            &shader,
            &layout,
            color_format,
            depth_format,
            sample_count,
            wgpu::ColorWrites::ALPHA,
            true,
        );
        let pipeline_none_depth = create_clear_pipeline(
            device,
            &shader,
            &layout,
            color_format,
            depth_format,
            sample_count,
            wgpu::ColorWrites::empty(),
            true,
        );
        let pipeline_all = create_clear_pipeline(
            device,
            &shader,
            &layout,
            color_format,
            depth_format,
            sample_count,
            wgpu::ColorWrites::ALL,
            false,
        );
        let pipeline_rgb = create_clear_pipeline(
            device,
            &shader,
            &layout,
            color_format,
            depth_format,
            sample_count,
            rgb,
            false,
        );
        let pipeline_alpha = create_clear_pipeline(
            device,
            &shader,
            &layout,
            color_format,
            depth_format,
            sample_count,
            wgpu::ColorWrites::ALPHA,
            false,
        );

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("efb_clear_uniforms"),
            size: std::mem::size_of::<ClearUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("efb_clear_bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        EfbClear {
            pipeline_all_depth,
            pipeline_rgb_depth,
            pipeline_alpha_depth,
            pipeline_none_depth,
            pipeline_all,
            pipeline_rgb,
            pipeline_alpha,
            uniform_buffer,
            bind_group,
        }
    }

    /// Clear a rectangular region with independent RGB, alpha, and depth masks.
    pub fn clear_region_masked(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        msaa_color_view: &wgpu::TextureView,
        resolve_color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        target_width: u32,
        target_height: u32,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        color: [f32; 4],
        depth: f32,
        color_update: bool,
        alpha_update: bool,
        z_update: bool,
    ) {
        if !color_update && !alpha_update && !z_update {
            return;
        }
        if w == 0 || h == 0 {
            tracing::warn!(x, y, w, h, "clear: zero-area clear region, skipping");
            return;
        }
        let x = x.min(target_width);
        let y = y.min(target_height);
        let w = w.min(target_width.saturating_sub(x));
        let h = h.min(target_height.saturating_sub(y));
        if w == 0 || h == 0 {
            tracing::warn!(x, y, w, h, "clear: zero-area after clamping to target, skipping");
            return;
        }

        // im gonna vomit
        let (pipeline, pipeline_label) = match (color_update, alpha_update, z_update) {
            (true, true, true) => (&self.pipeline_all_depth, "all+depth"),
            (true, false, true) => (&self.pipeline_rgb_depth, "rgb+depth"),
            (false, true, true) => (&self.pipeline_alpha_depth, "alpha+depth"),
            (false, false, true) => (&self.pipeline_none_depth, "depth-only"),
            (true, true, false) => (&self.pipeline_all, "all"),
            (true, false, false) => (&self.pipeline_rgb, "rgb"),
            (false, true, false) => (&self.pipeline_alpha, "alpha"),
            (false, false, false) => unreachable!(),
        };

        let uniforms = ClearUniforms {
            color,
            depth,
            _pad: [0.0; 3],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let group_label = format!(
            "EFB Clear rect=({},{} {}x{}) masks={}",
            x,
            y,
            w,
            h,
            clear_mask_label(color_update, alpha_update, z_update)
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("efb_clear_encoder"),
        });
        encoder.push_debug_group(&group_label);
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("efb_clear_region"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: if crate::EFB_SAMPLE_COUNT == 1 {
                        resolve_color_view
                    } else {
                        msaa_color_view
                    },
                    resolve_target: if crate::EFB_SAMPLE_COUNT == 1 {
                        None
                    } else {
                        Some(resolve_color_view)
                    },
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });

            rpass.set_pipeline(pipeline);
            let pipeline_marker = format!("Clear pipeline: {pipeline_label}");
            rpass.insert_debug_marker(&pipeline_marker);
            rpass.set_bind_group(0, &self.bind_group, &[]);
            rpass.set_scissor_rect(x, y, w, h);
            let values_marker = format!(
                "Clear values: color=({:.3},{:.3},{:.3},{:.3}) depth={:.6}",
                color[0], color[1], color[2], color[3], depth
            );
            rpass.insert_debug_marker(&values_marker);
            rpass.draw(0..3, 0..1);
        }
        encoder.pop_debug_group();
        queue.submit([encoder.finish()]);
    }
}
