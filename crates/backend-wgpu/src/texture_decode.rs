use crate::{GxRenderer, align_up};
use gecko::flipper::gx::draw::TextureFormat;
use gecko::flipper::gx::texture::{EncodedTexture, block_dims, mip_dimensions, raw_data_size};
use std::num::NonZeroU64;

const PARAMS_SIZE: u64 = size_of::<DecodeParams>() as u64;

pub(crate) struct TextureDecoder {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    offset_alignment: u64,
    max_binding_size: u64,
}

impl TextureDecoder {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gx_texture_decode"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/texture_decode.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gx_texture_decode"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(PARAMS_SIZE + 4),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gx_texture_decode"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gx_texture_decode"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("decode"),
            compilation_options: Default::default(),
            cache: None,
        });
        let limits = device.limits();

        Self {
            layout,
            pipeline,
            offset_alignment: u64::from(limits.min_storage_buffer_offset_alignment),
            max_binding_size: u64::from(limits.max_storage_buffer_binding_size),
        }
    }

    fn bind_group(
        &self,
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
        offset: u64,
        end: u64,
        texture: &wgpu::Texture,
        level: u32,
    ) -> wgpu::BindGroup {
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("gx_texture_decode_mip"),
            base_mip_level: level,
            mip_level_count: Some(1),
            usage: Some(wgpu::TextureUsages::STORAGE_BINDING),
            ..Default::default()
        });

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gx_texture_decode"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer,
                        offset,
                        size: NonZeroU64::new(end - offset),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DecodeParams {
    width: u32,
    height: u32,
    format: u32,
    tlut_format: u32,
    tile_w: u32,
    tile_h: u32,
    tile_bytes: u32,
    data_offset: u32,
    palette_offset: u32,
    palette_len: u32,
}

struct DecodeUpload {
    start: u64,
    header_stride: u64,
    bytes_start: u64,
    palette_start: u64,
    end: u64,
}

impl DecodeUpload {
    fn new(offset: u64, alignment: u64, levels: u32, data: &EncodedTexture) -> Self {
        let start = align_up(offset, alignment);
        let header_stride = align_up(PARAMS_SIZE, alignment);
        let bytes_start = start + header_stride * u64::from(levels);
        let palette_start = align_up(bytes_start + data.bytes.len() as u64, 4);

        Self {
            start,
            header_stride,
            bytes_start,
            palette_start,
            end: palette_start + data.palette.len() as u64 * 4,
        }
    }
}

impl GxRenderer {
    pub(crate) fn decode_texture_levels(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        format: TextureFormat,
        data: &EncodedTexture,
    ) {
        let alignment = self.texture_decoder.offset_alignment;
        let levels = texture.mip_level_count();

        let mut upload = DecodeUpload::new(self.texture_staging_scratch.len() as u64, alignment, levels, data);
        if upload.end > self.texture_staging_capacity {
            let _ = self.submit_pending(queue);
            self.grow_texture_staging(device, upload.end);
            upload = DecodeUpload::new(0, alignment, levels, data);
        }
        assert!(upload.end <= self.texture_staging_capacity);
        assert!(upload.end - upload.start <= self.texture_decoder.max_binding_size);

        let scratch = &mut self.texture_staging_scratch;
        scratch.resize(upload.bytes_start as usize, 0);
        scratch.extend_from_slice(&data.bytes);
        scratch.resize(upload.palette_start as usize, 0);
        scratch.extend(data.palette.iter().flat_map(|&entry| u32::from(entry).to_le_bytes()));

        let mut encoder = self.take_or_create_encoder(device);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gx_texture_decode"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.texture_decoder.pipeline);

        let tile = block_dims(format);
        let mut raw_offset = 0;
        for (level, (width, height)) in mip_dimensions(texture.width(), texture.height(), levels).enumerate() {
            let header = upload.start + level as u64 * upload.header_stride;
            let words_start = header + PARAMS_SIZE;
            let params = DecodeParams {
                width,
                height,
                format: format as u32,
                tlut_format: data.tlut_format as u32,
                tile_w: tile.tile_w,
                tile_h: tile.tile_h,
                tile_bytes: tile.bytes_per_tile,
                data_offset: (upload.bytes_start + raw_offset - words_start) as u32,
                palette_offset: ((upload.palette_start - words_start) / 4) as u32,
                palette_len: data.palette.len() as u32,
            };
            self.texture_staging_scratch[header as usize..words_start as usize]
                .copy_from_slice(bytemuck::bytes_of(&params));

            let bind_group = self.texture_decoder.bind_group(
                device,
                &self.texture_staging_buffer,
                header,
                upload.end,
                texture,
                level as u32,
            );
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);

            raw_offset += raw_data_size(width, height, format) as u64;
        }

        drop(pass);
        self.current_encoder = Some(encoder);
    }
}
