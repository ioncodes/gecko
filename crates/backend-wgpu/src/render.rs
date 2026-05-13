use crate::{GpuVertex, GxRenderer, PendingWriteback, align_up};
use gecko::common::Address;
use gecko::flipper::gx::texture::{self, CopyFormat};
use gecko::host::{EfbWriteback, XfbPart};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct XfbCopyUniforms {
    src_rect: [f32; 4],
    dst_size: [f32; 2],
    gamma: f32,
    filter_mode: u32,
}

pub(crate) struct EfbPackPipelines {
    pub(crate) rgba8: wgpu::RenderPipeline,
    pub(crate) rgba8_intensity: wgpu::RenderPipeline,
    pub(crate) i8: wgpu::RenderPipeline,
    pub(crate) i4: wgpu::RenderPipeline,
    pub(crate) ia8: wgpu::RenderPipeline,
    pub(crate) ia4: wgpu::RenderPipeline,
    pub(crate) rgb565: wgpu::RenderPipeline,
    pub(crate) rgb565_intensity: wgpu::RenderPipeline,
    pub(crate) rgb5a3: wgpu::RenderPipeline,
    pub(crate) rgb5a3_intensity: wgpu::RenderPipeline,
    pub(crate) a8: wgpu::RenderPipeline,
    pub(crate) r8: wgpu::RenderPipeline,
    pub(crate) rg8: wgpu::RenderPipeline,
}

impl EfbPackPipelines {
    pub(crate) fn for_color(&self, fmt: CopyFormat, intensity: bool) -> Option<&wgpu::RenderPipeline> {
        Some(match (fmt, intensity) {
            (CopyFormat::RGBA8, false) => &self.rgba8,
            (CopyFormat::RGBA8, true) => &self.rgba8_intensity,
            (CopyFormat::RGB565, false) => &self.rgb565,
            (CopyFormat::RGB565, true) => &self.rgb565_intensity,
            (CopyFormat::RGB5A3, false) => &self.rgb5a3,
            (CopyFormat::RGB5A3, true) => &self.rgb5a3_intensity,
            (CopyFormat::I8, _) => &self.i8,
            (CopyFormat::I4, _) => &self.i4,
            (CopyFormat::IA8, _) => &self.ia8,
            (CopyFormat::IA4, _) => &self.ia4,
            (CopyFormat::A8, _) => &self.a8,
            (CopyFormat::R8, _) => &self.r8,
            (CopyFormat::RG8, _) => &self.rg8,
            (CopyFormat::Z24X8, _) => return None,
        })
    }
}

/// Upload `bytes` into `buffer` at offset 0 via `write_buffer_with`, which
/// hands us a writable view into wgpu's staging memory and skips the extra
/// copy `write_buffer` would do. No-op when `bytes` is empty.
fn write_buffer(queue: &wgpu::Queue, buffer: &wgpu::Buffer, bytes: &[u8]) {
    let Some(size) = std::num::NonZeroU64::new(bytes.len() as u64) else {
        return;
    };
    let mut view = queue
        .write_buffer_with(buffer, 0, size)
        .expect("buffer too small for write_buffer_with");
    view.copy_from_slice(bytes);
}

impl GxRenderer {
    pub(crate) fn upload_buffers(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame_uniform_bytes: &[u8]) {
        let num_draws = self.scratch_draws.len();
        self.ensure_draw_capacity(device, num_draws);

        if self.scratch_vertices.len() > self.vertex_capacity {
            self.vertex_capacity = self.scratch_vertices.len().next_power_of_two();
            self.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gx_vertices"),
                size: (self.vertex_capacity * std::mem::size_of::<GpuVertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        if self.scratch_indices.len() > self.index_buffer_capacity {
            self.index_buffer_capacity = self.scratch_indices.len().next_power_of_two();
            self.index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gx_indices"),
                size: (self.index_buffer_capacity * std::mem::size_of::<u32>()) as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        let needed_frame_size = frame_uniform_bytes.len() as u64;
        if needed_frame_size > self.frame_uniform_buffer.size() {
            self.frame_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gx_frame_uniforms"),
                size: needed_frame_size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.bind_group_cache.clear();
        }

        write_buffer(queue, &self.frame_uniform_buffer, frame_uniform_bytes);
        write_buffer(queue, &self.draw_uniform_buffer, &self.scratch_uniform_bytes);
        write_buffer(queue, &self.vertex_buffer, bytemuck::cast_slice(&self.scratch_vertices));
        if !self.scratch_indices.is_empty() {
            write_buffer(queue, &self.index_buffer, bytemuck::cast_slice(&self.scratch_indices));
        }
    }

    pub(crate) fn execute_copy_xfb(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: u32,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        dst_h: u32,
        gamma: f32,
        clear: bool,
        clear_color: [f32; 4],
        clear_z: f32,
        color_update: bool,
        alpha_update: bool,
        z_update: bool,
        alpha_supported: bool,
    ) {
        let width = src_w.min(crate::EFB_WIDTH.saturating_sub(src_x));
        let height = src_h.min(crate::EFB_HEIGHT.saturating_sub(src_y));
        let dst_h = dst_h.max(1);
        if width == 0 || height == 0 {
            tracing::warn!(
                src_x,
                src_y,
                src_w,
                src_h,
                "efb_copy: zero-area region after clamping, skipping"
            );
            return;
        }

        let needs_shader_copy = dst_h != height || (gamma - 1.0).abs() > f32::EPSILON;
        if needs_shader_copy && self.xfb_copy_uniform_write_pending {
            self.submit_pending(queue);
        }

        let entry = self.xfb_copies.entry(id).or_insert_with(|| {
            let texture_label = format!("xfb_copy_tmp id={id} size={width}x{dst_h}");
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&texture_label),
                size: wgpu::Extent3d {
                    width,
                    height: dst_h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.surface_format,
                usage: wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            (tex, view)
        });

        // Recreate if size changed.
        let existing_size = entry.0.size();
        if existing_size.width != width || existing_size.height != dst_h {
            let texture_label = format!("xfb_copy_tmp id={id} size={width}x{dst_h}");
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&texture_label),
                size: wgpu::Extent3d {
                    width,
                    height: dst_h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.surface_format,
                usage: wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });

            let view = tex.create_view(&Default::default());
            *entry = (tex, view);
        }

        let group_label = format!(
            "CopyXfb id={id} src=({src_x},{src_y} {width}x{height}) dst_h={dst_h} gamma={gamma:.3} clear={clear}"
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("copy_xfb_encoder"),
        });
        encoder.push_debug_group(&group_label);
        if needs_shader_copy {
            encoder.insert_debug_marker("CopyXfb path: shader copy for scale/gamma");
            let uniforms = XfbCopyUniforms {
                src_rect: [src_x as f32, src_y as f32, width as f32, height as f32],
                dst_size: [width as f32, dst_h as f32],
                gamma,
                filter_mode: 0,
            };
            queue.write_buffer(&self.xfb_copy_uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("xfb_copy_bg"),
                layout: &self.xfb_copy_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.xfb_copy_uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.efb_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.xfb_copy_sampler),
                    },
                ],
            });

            {
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("xfb_copy"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &entry.1,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                });
                rpass.set_pipeline(&self.xfb_copy_pipeline);
                rpass.set_bind_group(0, &bind_group, &[]);
                let marker = format!(
                    "XFB shader uniforms: src=({src_x},{src_y} {width}x{height}) dst={width}x{dst_h} gamma={gamma:.3}"
                );
                rpass.insert_debug_marker(&marker);
                rpass.draw(0..3, 0..1);
            }
        } else {
            // Keep exact 1:1 XFB copies on the raw copy path. Running them
            // through the shader would sample with filtering and can soften
            // the image even when no scaling or gamma is requested.
            // TODO: We could just call it a trade-off and just have it all go
            // through? It looks a bit fuzzy, has it's own charm.
            encoder.insert_debug_marker("CopyXfb path: raw texture copy");
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.efb_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: src_x,
                        y: src_y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::default(),
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &entry.0,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::default(),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        encoder.pop_debug_group();

        self.pending_command_buffers.push(encoder.finish());
        if needs_shader_copy {
            self.xfb_copy_uniform_write_pending = true;
        }

        // Region-scoped EFB clear after copy (if requested).
        if clear {
            self.clear_efb_region(
                device,
                queue,
                src_x,
                src_y,
                src_w,
                src_h,
                clear_color,
                clear_z,
                color_update,
                alpha_update && alpha_supported,
                z_update,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn cache_efb_copy_color(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dest_addr: Address,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        half: bool,
        copy_format: CopyFormat,
        is_intensity: bool,
    ) {
        debug_assert!(
            self.efb_pack_pipelines.for_color(copy_format, is_intensity).is_some(),
            "cache_efb_copy_color called with depth-only copy format {copy_format:?}",
        );

        let width = src_w.min(crate::EFB_WIDTH.saturating_sub(src_x));
        let height = src_h.min(crate::EFB_HEIGHT.saturating_sub(src_y));
        if width == 0 || height == 0 {
            tracing::warn!(
                src_x,
                src_y,
                src_w,
                src_h,
                "efb_copy_cache: zero-area region after clamping, skipping"
            );
            return;
        }
        let divisor = if half { 2 } else { 1 };
        let dst_w = (width / divisor).max(1);
        let dst_h = (height / divisor).max(1);

        if let Some(entry) = self.efb_copy_cache.remove(&dest_addr) {
            self.return_to_pool(entry.texture, entry.view);
        }
        self.bind_group_cache
            .retain(|key, _| !key.tex_keys.iter().any(|k| k.map(|t| t.ram_addr) == Some(dest_addr)));

        let (tex, view) = self
            .efb_copy_pool
            .get_mut(&(dst_w, dst_h))
            .and_then(Vec::pop)
            .unwrap_or_else(|| {
                let label = format!("efb_copy addr={dest_addr:#010x} size={dst_w}x{dst_h} fmt={copy_format:?}");
                let tex = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(&label),
                    size: wgpu::Extent3d {
                        width: dst_w,
                        height: dst_h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = tex.create_view(&Default::default());
                (tex, view)
            });

        if self.xfb_copy_uniform_write_pending {
            self.submit_pending(queue);
        }

        let uniforms = XfbCopyUniforms {
            src_rect: [src_x as f32, src_y as f32, width as f32, height as f32],
            dst_size: [dst_w as f32, dst_h as f32],
            gamma: 1.0,
            filter_mode: 0,
        };
        queue.write_buffer(&self.xfb_copy_uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("efb_pack_bg"),
            layout: &self.xfb_copy_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.xfb_copy_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.efb_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.xfb_copy_sampler),
                },
            ],
        });

        let group_label = format!(
            "CopyEfbToTexture addr={dest_addr:#010x} src=({src_x},{src_y} {width}x{height}) dst={dst_w}x{dst_h} fmt={copy_format:?}"
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("efb_pack_encoder"),
        });
        encoder.push_debug_group(&group_label);
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("efb_pack"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            rpass.set_pipeline(self.efb_pack_pipelines.for_color(copy_format, is_intensity).unwrap());
            rpass.set_bind_group(0, &bind_group, &[]);
            rpass.insert_debug_marker("EFB copy: per-format pack into cache");
            rpass.draw(0..3, 0..1);
        }
        encoder.pop_debug_group();
        self.pending_command_buffers.push(encoder.finish());
        self.xfb_copy_uniform_write_pending = true;

        self.efb_copy_cache.insert(
            dest_addr,
            crate::EfbCopyEntry {
                format: copy_format,
                texture: tex,
                view,
            },
        );
    }

    pub(crate) fn return_to_pool(&mut self, tex: wgpu::Texture, view: wgpu::TextureView) {
        const PER_BUCKET_CAP: usize = 8;
        let size = tex.size();
        let bucket = self.efb_copy_pool.entry((size.width, size.height)).or_default();
        if bucket.len() < PER_BUCKET_CAP {
            bucket.push((tex, view));
        }
    }

    pub(crate) fn return_load_texture_to_pool(&mut self, tex: wgpu::Texture) {
        const PER_BUCKET_CAP: usize = 8;

        debug_assert_eq!(tex.format(), wgpu::TextureFormat::Rgba8Unorm);
        debug_assert_eq!(tex.mip_level_count(), 1);
        debug_assert_eq!(tex.sample_count(), 1);
        debug_assert_eq!(tex.dimension(), wgpu::TextureDimension::D2);
        debug_assert!(tex.usage().contains(
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        ));

        let size = tex.size();
        let bucket = self.texture_pool.entry((size.width, size.height)).or_default();
        if bucket.len() < PER_BUCKET_CAP {
            bucket.push(tex);
        }
    }

    pub(crate) fn execute_present_xfb(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        parts: &[XfbPart],
    ) {
        let width = width.max(1);
        let height = height.max(1);

        // Resize the XFB output texture if the frame dimensions changed.
        let cur = self.xfb_texture.size();
        if cur.width != width || cur.height != height {
            let texture_label = format!("xfb_accum size={width}x{height}");
            self.xfb_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&texture_label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.surface_format,
                usage: wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.xfb_view = self.xfb_texture.create_view(&Default::default());
        }

        let group_label = format!("PresentXfb size={width}x{height} parts={}", parts.len());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("present_xfb_encoder"),
        });
        encoder.push_debug_group(&group_label);

        // Don't clear the XFB: let previous content persist so partial
        // frames show the last valid content instead of a black flash.

        let xfb_size = self.xfb_texture.size();

        for part in parts {
            let Some((tex, _)) = self.xfb_copies.get(&part.id) else {
                tracing::warn!(id = part.id, "present_xfb: XFB copy not found in cache, skipping part");
                let marker = format!("PresentXfb skip: missing part id={}", part.id);
                encoder.insert_debug_marker(&marker);
                continue;
            };
            let src_size = tex.size();
            let width = src_size.width.min(xfb_size.width.saturating_sub(part.offset_x));
            let height = src_size.height.min(xfb_size.height.saturating_sub(part.offset_y));
            if width == 0 || height == 0 {
                tracing::warn!(id = part.id, "present_xfb: zero-area XFB part after clamping, skipping");
                let marker = format!("PresentXfb skip: zero-area part id={}", part.id);
                encoder.insert_debug_marker(&marker);
                continue;
            }
            let marker = format!(
                "XFB part id={} dst=({},{} {}x{})",
                part.id, part.offset_x, part.offset_y, width, height
            );
            encoder.insert_debug_marker(&marker);
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::default(),
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.xfb_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: part.offset_x,
                        y: part.offset_y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::default(),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }

        encoder.pop_debug_group();
        self.pending_command_buffers.push(encoder.finish());
        self.submit_pending(queue);
        self.xfb_has_content = true;
    }

    /// Queue an EFB region readback into `pending_writebacks`. The actual
    /// map+encode+ship happens at the next frame boundary via
    /// `drain_pending_writebacks`.
    pub(crate) fn execute_copy_efb_to_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dest_addr: Address,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        copy_format: u8,
        mipmap: bool,
        stride: u32,
        depth_copy: bool,
    ) {
        // Clamp the source to EFB bounds (mirrors execute_copy_xfb).
        let width = src_w.min(crate::EFB_WIDTH.saturating_sub(src_x));
        let height = src_h.min(crate::EFB_HEIGHT.saturating_sub(src_y));
        if width == 0 || height == 0 {
            tracing::warn!(
                src_x,
                src_y,
                src_w,
                src_h,
                "efb_to_texture: zero-area region after clamping, skipping"
            );
            return;
        }

        let copy_format_option = if depth_copy {
            texture::CopyFormat::from_u8_depth(copy_format)
        } else {
            texture::CopyFormat::from_u8_color(copy_format)
        };
        let Some(copy_format_enum) = copy_format_option else {
            tracing::warn!(
                copy_format = format!("{copy_format:#x}"),
                "efb_to_texture: unsupported copy format, skipping readback"
            );
            return;
        };

        if depth_copy {
            self.execute_depth_writeback(
                device,
                queue,
                dest_addr,
                src_x,
                src_y,
                width,
                height,
                mipmap,
                stride,
                copy_format_enum,
            );
            return;
        }

        // wgpu requires 256-byte row alignment for texture<->buffer copies.
        let bytes_per_row = align_up(width as u64 * 4, 256);
        let staging_size = bytes_per_row * height as u64;
        let (staging, staging_capacity) = self.acquire_readback_staging(device, staging_size);

        let group_label = format!(
            "CopyEfbToTexture addr={dest_addr:#010x} src=({src_x},{src_y} {width}x{height}) fmt={copy_format_enum:?} mip={mipmap} stride={stride} depth={depth_copy}"
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("efb_to_texture_copy_encoder"),
        });
        encoder.push_debug_group(&group_label);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.efb_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: src_x,
                    y: src_y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::default(),
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row as u32),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        encoder.pop_debug_group();
        self.pending_command_buffers.push(encoder.finish());

        let swap_bgra = matches!(
            self.surface_format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        self.pending_writebacks.push(PendingWriteback {
            dest_addr,
            staging,
            staging_capacity,
            bytes_per_row,
            staging_size,
            width,
            height,
            copy_format: copy_format_enum,
            stride,
            swap_bgra,
            box_filter_downsample: mipmap,
            label: "efb_to_texture",
        });
    }

    fn execute_depth_writeback(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dest_addr: Address,
        src_x: u32,
        src_y: u32,
        width: u32,
        height: u32,
        mipmap: bool,
        stride: u32,
        copy_format_enum: texture::CopyFormat,
    ) {
        let divisor = if mipmap { 2 } else { 1 };
        let encode_w = (width / divisor).max(1);
        let encode_h = (height / divisor).max(1);

        let (target_w, target_h) = self
            .efb_depth_writeback_target
            .as_ref()
            .map(|(t, _)| (t.size().width, t.size().height))
            .unwrap_or((0, 0));
        if self.efb_depth_writeback_target.is_none() || encode_w > target_w || encode_h > target_h {
            let new_w = target_w.max(encode_w).max(64);
            let new_h = target_h.max(encode_h).max(64);
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("efb_depth_writeback_target"),
                size: wgpu::Extent3d {
                    width: new_w,
                    height: new_h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            self.efb_depth_writeback_target = Some((tex, view));
        }

        if self.efb_depth_resolve_uniform_write_pending {
            self.submit_pending(queue);
        }

        let uniforms = XfbCopyUniforms {
            src_rect: [src_x as f32, src_y as f32, width as f32, height as f32],
            dst_size: [encode_w as f32, encode_h as f32],
            gamma: 1.0,
            filter_mode: 0,
        };
        queue.write_buffer(&self.efb_depth_resolve_uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
        self.efb_depth_resolve_uniform_write_pending = true;

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("efb_depth_bg"),
            layout: &self.efb_depth_resolve_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.efb_depth_resolve_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.efb_depth_view),
                },
            ],
        });

        let bytes_per_row = align_up(encode_w as u64 * 4, 256);
        let staging_size = bytes_per_row * encode_h as u64;
        let (staging, staging_capacity) = self.acquire_readback_staging(device, staging_size);

        let (writeback_tex, writeback_view) = self.efb_depth_writeback_target.as_ref().unwrap();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("efb_depth_encoder"),
        });
        encoder.push_debug_group(&format!(
            "EfbDepth addr={dest_addr:#010x} src=({src_x},{src_y} {width}x{height}) dst={encode_w}x{encode_h}"
        ));
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("efb_depth_writeback_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: writeback_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            rpass.set_viewport(0.0, 0.0, encode_w as f32, encode_h as f32, 0.0, 1.0);
            rpass.set_scissor_rect(0, 0, encode_w, encode_h);
            rpass.set_pipeline(&self.efb_depth_writeback_pipeline);
            rpass.set_bind_group(0, &bind_group, &[]);
            rpass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: writeback_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::default(),
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row as u32),
                    rows_per_image: Some(encode_h),
                },
            },
            wgpu::Extent3d {
                width: encode_w,
                height: encode_h,
                depth_or_array_layers: 1,
            },
        );
        encoder.pop_debug_group();
        self.pending_command_buffers.push(encoder.finish());
        self.efb_depth_resolve_uniform_write_pending = false;

        self.pending_writebacks.push(PendingWriteback {
            dest_addr,
            staging,
            staging_capacity,
            bytes_per_row,
            staging_size,
            width: encode_w,
            height: encode_h,
            copy_format: copy_format_enum,
            stride,
            swap_bgra: false,
            box_filter_downsample: false,
            label: "efb_depth_writeback",
        });
    }

    pub(crate) fn acquire_readback_staging(&mut self, device: &wgpu::Device, staging_size: u64) -> (wgpu::Buffer, u64) {
        let capacity = staging_size.next_power_of_two().max(4096);
        if let Some(bucket) = self.efb_readback_staging_pool.get_mut(&capacity) {
            if let Some(buf) = bucket.pop() {
                return (buf, capacity);
            }
        }

        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("efb_readback_staging"),
            size: capacity,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        (buf, capacity)
    }

    pub(crate) fn return_readback_staging(&mut self, buf: wgpu::Buffer, capacity: u64) {
        const MAX_PER_BUCKET: usize = 8;
        let bucket = self.efb_readback_staging_pool.entry(capacity).or_default();
        if bucket.len() < MAX_PER_BUCKET {
            bucket.push(buf);
        }
    }

    pub(crate) fn drain_pending_writebacks(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.pending_writebacks.is_empty() {
            return;
        }

        self.submit_pending(queue);

        for pending in &self.pending_writebacks {
            pending
                .staging
                .slice(..pending.staging_size)
                .map_async(wgpu::MapMode::Read, |_| {});
        }

        if let Err(err) = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(5)),
        }) {
            tracing::warn!(?err, "efb writeback drain: device poll failed");
            // Best-effort: drop the buffers back into the pool so we don't leak.
            let pending: Vec<PendingWriteback> = self.pending_writebacks.drain(..).collect();
            for w in pending {
                self.return_readback_staging(w.staging, w.staging_capacity);
            }
            return;
        }

        let pending: Vec<PendingWriteback> = self.pending_writebacks.drain(..).collect();
        for w in pending {
            let mut rgba = vec![0u8; (w.width * w.height * 4) as usize];
            {
                let slice = w.staging.slice(..w.staging_size);
                let mapped = slice.get_mapped_range();
                let row_bytes = (w.width * 4) as usize;
                let src_row_bytes = w.bytes_per_row as usize;

                for y in 0..w.height as usize {
                    let src_row = &mapped[y * src_row_bytes..y * src_row_bytes + row_bytes];
                    let dst_row = &mut rgba[y * row_bytes..y * row_bytes + row_bytes];

                    if w.swap_bgra {
                        for i in 0..w.width as usize {
                            dst_row[i * 4] = src_row[i * 4 + 2];
                            dst_row[i * 4 + 1] = src_row[i * 4 + 1];
                            dst_row[i * 4 + 2] = src_row[i * 4];
                            dst_row[i * 4 + 3] = src_row[i * 4 + 3];
                        }
                    } else {
                        dst_row.copy_from_slice(src_row);
                    }
                }
            }
            w.staging.unmap();

            let (encode_w, encode_h, encode_src) = if w.box_filter_downsample {
                (
                    w.width / 2,
                    w.height / 2,
                    texture::downsample_box_2x(&rgba, w.width, w.height),
                )
            } else {
                (w.width, w.height, rgba)
            };

            let encoded = texture::encode_from_rgba(&encode_src, encode_w as usize, encode_h as usize, w.copy_format);
            let row_bytes = texture::encoded_row_bytes(encode_w, w.copy_format);
            let row_count = texture::encoded_row_count(encode_h, w.copy_format);
            let dest_stride_bytes = w.stride as usize;

            if let Some(tx) = &self.efb_writeback_tx {
                if let Err(err) = tx.try_send(EfbWriteback {
                    dest_addr: w.dest_addr,
                    bytes: encoded,
                    row_bytes,
                    row_count,
                    dest_stride_bytes,
                }) {
                    tracing::warn!(?err, label = w.label, "writeback channel send failed");
                }
            }

            self.return_readback_staging(w.staging, w.staging_capacity);
        }
    }

    fn ensure_draw_capacity(&mut self, device: &wgpu::Device, count: usize) {
        if count <= self.draw_uniform_capacity {
            return;
        }

        self.draw_uniform_capacity = count.next_power_of_two();
        self.draw_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx_draw_uniforms"),
            size: self.draw_uniform_stride * self.draw_uniform_capacity as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.bind_group_cache.clear();
    }
}
