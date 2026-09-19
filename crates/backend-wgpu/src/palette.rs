use gecko::flipper::gx::texture::CopyFormat;

pub(crate) struct PaletteConverter {
    layout: wgpu::BindGroupLayout,
    i8: wgpu::RenderPipeline,
    r8: wgpu::RenderPipeline,
}

impl PaletteConverter {
    pub(crate) fn new(
        device: &wgpu::Device,
        pack_formats_wgsl: &str,
        make_pipeline: impl Fn(&str, &str, &wgpu::PipelineLayout, &wgpu::ShaderModule) -> wgpu::RenderPipeline,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("efb_palette"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{}\n{}", include_str!("shaders/efb_palette.wgsl"), pack_formats_wgsl).into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("efb_palette"),
            entries: &std::array::from_fn::<_, 2, _>(|binding| wgpu::BindGroupLayoutEntry {
                binding: binding as u32,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("efb_palette"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        Self {
            i8: make_pipeline("efb_palette_i8", "fs_palette_i8", &pipeline_layout, &shader),
            r8: make_pipeline("efb_palette_r8", "fs_palette_r8", &pipeline_layout, &shader),
            layout,
        }
    }

    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        format: CopyFormat,
        palette: &[[u8; 4]; 256],
        target: &wgpu::TextureView,
    ) {
        let palette_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("efb_ci8_palette"),
            size: wgpu::Extent3d {
                width: 256,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        queue.write_texture(
            palette_texture.as_image_copy(),
            palette.as_flattened(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(1024),
                rows_per_image: None,
            },
            palette_texture.size(),
        );
        let source_view = source.create_view(&Default::default());
        let palette_view = palette_texture.create_view(&Default::default());
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("efb_ci8_palette"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&palette_view),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("efb_ci8_palette"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_pipeline(match format {
            CopyFormat::I8 => &self.i8,
            CopyFormat::R8 => &self.r8,
            _ => panic!("unsupported CI8 source format"),
        });
        pass.set_bind_group(0, &bindings, &[]);
        pass.draw(0..3, 0..1);
    }
}
