use crate::{GxRenderer, PendingWriteback, align_up, compute_draw_buffer_layout};
use gecko::common::Address;
use gecko::flipper::gx::texture::{self, CopyFormat};
use gecko::host::XfbPart;

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

pub(crate) struct EfbDepthPackPipelines {
    pub(crate) z24x8: wgpu::RenderPipeline,
    pub(crate) i8: wgpu::RenderPipeline,
    pub(crate) i4: wgpu::RenderPipeline,
    pub(crate) ia8: wgpu::RenderPipeline,
    pub(crate) ia4: wgpu::RenderPipeline,
    pub(crate) rgb565: wgpu::RenderPipeline,
    pub(crate) rgb5a3: wgpu::RenderPipeline,
    pub(crate) a8: wgpu::RenderPipeline,
    pub(crate) r8: wgpu::RenderPipeline,
    pub(crate) rg8: wgpu::RenderPipeline,
}

#[derive(Clone, Copy)]
enum EfbCopySource {
    Color { intensity: bool },
    Depth,
}

impl EfbDepthPackPipelines {
    pub(crate) fn for_format(&self, fmt: CopyFormat) -> &wgpu::RenderPipeline {
        match fmt {
            CopyFormat::RGBA8 | CopyFormat::Z24X8 => &self.z24x8,
            CopyFormat::I8 => &self.i8,
            CopyFormat::I4 => &self.i4,
            CopyFormat::IA8 => &self.ia8,
            CopyFormat::IA4 => &self.ia4,
            CopyFormat::RGB565 => &self.rgb565,
            CopyFormat::RGB5A3 => &self.rgb5a3,
            CopyFormat::A8 => &self.a8,
            CopyFormat::R8 => &self.r8,
            CopyFormat::RG8 => &self.rg8,
        }
    }
}

impl GxRenderer {
    pub(crate) fn upload_buffers(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame_uniform_bytes: &[u8]) {
        let num_draws = self.scratch_draws.len();
        self.ensure_draw_capacity(num_draws);

        // Total packed vertex bytes = sum of per-draw (vertex_count * stride).
        // Variable per draw because stride depends on `active_texcoords`.
        let vertex_used: u64 = self
            .scratch_draws
            .iter()
            .map(|d| u64::from(d.vertex_count) * u64::from(d.packed_vertex_stride))
            .sum();
        let frame_used = frame_uniform_bytes.len() as u64;
        let draw_used = self.scratch_uniform_bytes.len() as u64;
        let index_used = (self.scratch_indices.len() * std::mem::size_of::<u32>()) as u64;

        let layout = self.draw_buffer_layout;
        let needs_grow = frame_used > layout.frame_capacity
            || draw_used > layout.draw_capacity
            || vertex_used > layout.vertex_capacity
            || index_used > layout.index_capacity;
        if needs_grow {
            let frame_cap = grow_capacity(layout.frame_capacity, frame_used);
            let draw_cap = grow_capacity(layout.draw_capacity, draw_used);
            let vertex_cap = grow_capacity(layout.vertex_capacity, vertex_used);
            let index_cap = grow_capacity(layout.index_capacity, index_used);
            self.draw_buffer_layout =
                compute_draw_buffer_layout(self.uniform_alignment, frame_cap, draw_cap, vertex_cap, index_cap);
            self.draw_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gx_draw_buffer"),
                size: self.draw_buffer_layout.total_size,
                usage: wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::UNIFORM
                    | wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::INDEX,
                mapped_at_creation: false,
            });
            // Bind groups embed BufferBinding pointers to the now-stale buffer.
            self.bind_group_cache.clear();
        }

        let layout = self.draw_buffer_layout;
        let total_used = layout.index_offset + index_used;
        let Some(size) = std::num::NonZeroU64::new(total_used) else {
            return;
        };
        let mut view = queue
            .write_buffer_with(&self.draw_buffer, 0, size)
            .expect("gx_draw_buffer too small for write_buffer_with");
        let frame_off = layout.frame_offset as usize;
        let draw_off = layout.draw_offset as usize;
        let vertex_off = layout.vertex_offset as usize;
        let index_off = layout.index_offset as usize;
        view.slice(frame_off..frame_off + frame_uniform_bytes.len())
            .copy_from_slice(frame_uniform_bytes);
        view.slice(draw_off..draw_off + self.scratch_uniform_bytes.len())
            .copy_from_slice(&self.scratch_uniform_bytes);

        for draw in &self.scratch_draws {
            let stride = draw.packed_vertex_stride as usize;
            let texcoord_bytes = stride - 68;
            let mut cursor = vertex_off + draw.packed_vertex_byte_offset as usize;
            let src_base = draw.src_vertex_index as usize;
            let src_end = src_base + draw.vertex_count as usize;
            for src_v in &self.scratch_vertices[src_base..src_end] {
                let src_bytes = bytemuck::bytes_of(src_v);

                view.slice(cursor..cursor + 68).copy_from_slice(&src_bytes[..68]);
                if texcoord_bytes > 0 {
                    view.slice(cursor + 68..cursor + 68 + texcoord_bytes)
                        .copy_from_slice(&src_bytes[68..68 + texcoord_bytes]);
                }

                cursor += stride;
            }
        }

        if !self.scratch_indices.is_empty() {
            view.slice(index_off..index_off + index_used as usize)
                .copy_from_slice(bytemuck::cast_slice(&self.scratch_indices));
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

        let scaled_src_x = self.scaled(src_x);
        let scaled_src_y = self.scaled(src_y);
        let scaled_w = self.scaled(width);
        let scaled_h = self.scaled(height);
        let scaled_dst_h = self.scaled(dst_h);

        let needs_shader_copy = dst_h != height || (gamma - 1.0).abs() > f32::EPSILON;
        if needs_shader_copy && self.xfb_copy_uniform_write_pending {
            let _ = self.submit_pending(queue);
        }

        let mut encoder = self.take_or_create_encoder(device);

        let entry = self.xfb_copies.entry(id).or_insert_with(|| {
            let texture_label = format!("xfb_copy_tmp id={id} size={scaled_w}x{scaled_dst_h}");
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&texture_label),
                size: wgpu::Extent3d {
                    width: scaled_w,
                    height: scaled_dst_h,
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
        if existing_size.width != scaled_w || existing_size.height != scaled_dst_h {
            let texture_label = format!("xfb_copy_tmp id={id} size={scaled_w}x{scaled_dst_h}");
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&texture_label),
                size: wgpu::Extent3d {
                    width: scaled_w,
                    height: scaled_dst_h,
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
        encoder.push_debug_group(&group_label);
        if needs_shader_copy {
            encoder.insert_debug_marker("CopyXfb path: shader copy for scale/gamma");
            let uniforms = XfbCopyUniforms {
                src_rect: [
                    scaled_src_x as f32,
                    scaled_src_y as f32,
                    scaled_w as f32,
                    scaled_h as f32,
                ],
                dst_size: [scaled_w as f32, scaled_dst_h as f32],
                gamma,
                filter_mode: 0,
            };
            queue.write_buffer(&self.xfb_copy_uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

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
                rpass.set_bind_group(0, &self.xfb_copy_bind_group, &[]);
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
                        x: scaled_src_x,
                        y: scaled_src_y,
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
                    width: scaled_w,
                    height: scaled_h,
                    depth_or_array_layers: 1,
                },
            );
        }
        encoder.pop_debug_group();

        self.current_encoder = Some(encoder);
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
    fn cache_efb_copy(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dest_addr: Address,
        src_x: u32,
        src_y: u32,
        width: u32,
        height: u32,
        half: bool,
        copy_format: CopyFormat,
        source: EfbCopySource,
    ) {
        if let EfbCopySource::Color { intensity } = source {
            debug_assert!(
                self.efb_pack_pipelines.for_color(copy_format, intensity).is_some(),
                "cache_efb_copy called with depth-only copy format {copy_format:?}",
            );
        }

        let divisor = if half { 2 } else { 1 };
        let dst_w = (width / divisor).max(1);
        let dst_h = (height / divisor).max(1);
        let scaled_dst_w = self.scaled(dst_w);
        let scaled_dst_h = self.scaled(dst_h);
        let (tex, view) = self.acquire_efb_copy_target(device, dest_addr, scaled_dst_w, scaled_dst_h, copy_format);

        let write_pending = match source {
            EfbCopySource::Color { .. } => self.xfb_copy_uniform_write_pending,
            EfbCopySource::Depth => self.efb_depth_cache_uniform_write_pending,
        };
        if write_pending {
            let _ = self.submit_pending(queue);
        }

        let uniforms = XfbCopyUniforms {
            src_rect: self.scaled_src_rect(src_x, src_y, width, height),
            dst_size: [scaled_dst_w as f32, scaled_dst_h as f32],
            gamma: 1.0,
            filter_mode: match source {
                EfbCopySource::Color { .. } => u32::from(half),
                EfbCopySource::Depth => 0,
            },
        };
        let uniform_buffer = match source {
            EfbCopySource::Color { .. } => &self.xfb_copy_uniform_buffer,
            EfbCopySource::Depth => &self.efb_depth_cache_uniform_buffer,
        };
        queue.write_buffer(uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let kind = match source {
            EfbCopySource::Color { .. } => "CopyEfbToTexture",
            EfbCopySource::Depth => "CopyEfbDepthToTexture",
        };
        let group_label = format!(
            "{kind} addr={dest_addr:#010x} src=({src_x},{src_y} {width}x{height}) dst={dst_w}x{dst_h} fmt={copy_format:?}"
        );
        let mut encoder = self.take_or_create_encoder(device);
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
            let (pipeline, bind_group) = match source {
                EfbCopySource::Color { intensity } => (
                    self.efb_pack_pipelines.for_color(copy_format, intensity).unwrap(),
                    &self.xfb_copy_bind_group,
                ),
                EfbCopySource::Depth => (
                    self.efb_depth_pack_pipelines.for_format(copy_format),
                    &self.efb_depth_cache_bind_group,
                ),
            };
            rpass.set_pipeline(pipeline);
            rpass.set_bind_group(0, bind_group, &[]);
            rpass.insert_debug_marker("EFB copy: per-format pack into cache");
            rpass.draw(0..3, 0..1);
        }
        encoder.pop_debug_group();
        self.current_encoder = Some(encoder);
        match source {
            EfbCopySource::Color { .. } => self.xfb_copy_uniform_write_pending = true,
            EfbCopySource::Depth => self.efb_depth_cache_uniform_write_pending = true,
        }

        self.efb_copy_cache.insert(
            dest_addr,
            crate::EfbCopyEntry {
                format: copy_format,
                native_w: dst_w,
                native_h: dst_h,
                texture: tex,
                view,
            },
        );
    }

    fn acquire_efb_copy_target(
        &mut self,
        device: &wgpu::Device,
        dest_addr: Address,
        width: u32,
        height: u32,
        copy_format: CopyFormat,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        if let Some(entry) = self.efb_copy_cache.remove(&dest_addr) {
            self.return_to_pool(entry.texture, entry.view);
        }
        self.bind_group_cache
            .retain(|key, _| !key.tex_keys.iter().any(|k| k.map(|t| t.ram_addr) == Some(dest_addr)));

        self.efb_copy_pool
            .get_mut(&(width, height))
            .and_then(Vec::pop)
            .unwrap_or_else(|| {
                let label = format!("efb_copy addr={dest_addr:#010x} size={width}x{height} fmt={copy_format:?}");
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(&label),
                    size: wgpu::Extent3d {
                        width,
                        height,
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
                let view = texture.create_view(&Default::default());
                (texture, view)
            })
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

    fn ensure_xfb_texture(&mut self, device: &wgpu::Device, width: u32, height: u32, label_prefix: &str) {
        let cur = self.xfb_texture.size();
        if cur.width == width && cur.height == height {
            return;
        }

        let texture_label = format!("{label_prefix} size={width}x{height}");
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

    pub(crate) fn execute_present_xfb(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        parts: &[XfbPart],
    ) {
        let width = self.scaled(width.max(1));
        let height = self.scaled(height.max(1));

        self.ensure_xfb_texture(device, width, height, "xfb_accum");

        let group_label = format!("PresentXfb size={width}x{height} parts={}", parts.len());
        let mut encoder = self.take_or_create_encoder(device);
        encoder.push_debug_group(&group_label);

        // Don't clear the XFB: let previous content persist so partial
        // frames show the last valid content instead of a black flash.

        let xfb_size = self.xfb_texture.size();

        for part in parts {
            let Some((tex, _)) = self.xfb_copies.get(&part.id) else {
                // Expected when the VI scans a buffer we've never snapshotted
                // (e.g. before the first copy to it); the previous frame is held.
                tracing::debug!(id = part.id, "present_xfb: XFB copy not found in cache, skipping part");
                let marker = format!("PresentXfb skip: missing part id={}", part.id);
                encoder.insert_debug_marker(&marker);
                continue;
            };

            let src_size = tex.size();
            let offset_x = self.scaled(part.offset_x);
            let offset_y = self.scaled(part.offset_y);
            let width = src_size.width.min(xfb_size.width.saturating_sub(offset_x));
            let height = src_size.height.min(xfb_size.height.saturating_sub(offset_y));
            if width == 0 || height == 0 {
                tracing::warn!(id = part.id, "present_xfb: zero-area XFB part after clamping, skipping");
                let marker = format!("PresentXfb skip: zero-area part id={}", part.id);
                encoder.insert_debug_marker(&marker);
                continue;
            }

            let marker = format!(
                "XFB part id={} dst=({},{} {}x{})",
                part.id, offset_x, offset_y, width, height
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
                        x: offset_x,
                        y: offset_y,
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
        self.current_encoder = Some(encoder);
        let _ = self.submit_pending(queue);
        // The staging buffer's in-flight references have just been submitted;
        // now's the safe moment to grow it if this frame exceeded capacity.
        self.maybe_grow_texture_staging(device);
        self.xfb_has_content = true;
    }

    pub(crate) fn execute_present_raw_xfb(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        pixels: &[u32],
    ) {
        let width = width.max(1);
        let height = height.max(1);

        if (pixels.len() as u64) < width as u64 * height as u64 {
            tracing::warn!(
                width,
                height,
                len = pixels.len(),
                "present_raw_xfb: pixel buffer too small, skipping"
            );
            return;
        }

        // The raw path never upscales: these pixels are the final image.
        self.ensure_xfb_texture(device, width, height, "xfb_raw");

        queue.write_texture(
            self.xfb_texture.as_image_copy(),
            bytemuck::cast_slice(&pixels[..(width as usize * height as usize)]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let _ = self.submit_pending(queue);
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
        is_intensity: bool,
    ) {
        tracing::debug!(
            dest_addr = format!("{dest_addr:#010X}"),
            src_x,
            src_y,
            src_w,
            src_h,
            copy_format,
            mipmap,
            depth_copy,
            "efb_to_texture copy"
        );
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
            self.cache_efb_copy(
                device,
                queue,
                dest_addr,
                src_x,
                src_y,
                width,
                height,
                mipmap,
                copy_format_enum,
                EfbCopySource::Depth,
            );
            return;
        }

        let readback_width = if mipmap { (width / 2).max(1) } else { width };
        let readback_height = if mipmap { (height / 2).max(1) } else { height };
        self.discard_superseded_writebacks(dest_addr, readback_width, readback_height, copy_format_enum, stride);

        let bytes_per_row = align_up(readback_width as u64 * 4, 256);
        let staging_size = bytes_per_row * readback_height as u64;
        let (staging, staging_capacity) = self.acquire_readback_staging(device, staging_size);

        let group_label = format!(
            "CopyEfbToTexture addr={dest_addr:#010x} src=({src_x},{src_y} {width}x{height}) fmt={copy_format_enum:?} mip={mipmap} stride={stride} depth={depth_copy}"
        );

        if self.efb_scale > 1 || mipmap {
            if self.efb_color_readback_uniform_write_pending {
                let _ = self.submit_pending(queue);
            }

            Self::ensure_writeback_target(
                device,
                &mut self.efb_color_readback_target,
                readback_width,
                readback_height,
                self.surface_format,
                "efb_color_readback_target",
            );

            let uniforms = XfbCopyUniforms {
                src_rect: self.scaled_src_rect(src_x, src_y, width, height),
                dst_size: [readback_width as f32, readback_height as f32],
                gamma: 1.0,
                filter_mode: if mipmap { 2 } else { 0 },
            };
            queue.write_buffer(
                &self.efb_color_readback_uniform_buffer,
                0,
                bytemuck::bytes_of(&uniforms),
            );
        }

        let mut encoder = self.take_or_create_encoder(device);
        encoder.push_debug_group(&group_label);

        let (src_texture, src_origin) = if self.efb_scale == 1 && !mipmap {
            (
                &self.efb_texture,
                wgpu::Origin3d {
                    x: src_x,
                    y: src_y,
                    z: 0,
                },
            )
        } else {
            let (readback_tex, readback_view) = self.efb_color_readback_target.as_ref().unwrap();

            {
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("efb_color_readback_downsample"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: readback_view,
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
                rpass.set_viewport(0.0, 0.0, readback_width as f32, readback_height as f32, 0.0, 1.0);
                rpass.set_scissor_rect(0, 0, readback_width, readback_height);
                rpass.set_pipeline(&self.xfb_copy_pipeline);
                rpass.set_bind_group(0, &self.efb_color_readback_bind_group, &[]);
                rpass.draw(0..3, 0..1);
            }

            (readback_tex, wgpu::Origin3d::ZERO)
        };

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: src_texture,
                mip_level: 0,
                origin: src_origin,
                aspect: wgpu::TextureAspect::default(),
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row as u32),
                    rows_per_image: Some(readback_height),
                },
            },
            wgpu::Extent3d {
                width: readback_width,
                height: readback_height,
                depth_or_array_layers: 1,
            },
        );
        encoder.pop_debug_group();
        self.current_encoder = Some(encoder);

        if self.efb_scale > 1 || mipmap {
            self.efb_color_readback_uniform_write_pending = true;
        }

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
            width: readback_width,
            height: readback_height,
            copy_format: copy_format_enum,
            stride,
            swap_bgra,
        });

        self.cache_efb_copy(
            device,
            queue,
            dest_addr,
            src_x,
            src_y,
            width,
            height,
            mipmap,
            copy_format_enum,
            EfbCopySource::Color {
                intensity: is_intensity,
            },
        );
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
        self.discard_superseded_writebacks(dest_addr, encode_w, encode_h, copy_format_enum, stride);

        Self::ensure_writeback_target(
            device,
            &mut self.efb_depth_writeback_target,
            encode_w,
            encode_h,
            wgpu::TextureFormat::Rgba8Unorm,
            "efb_depth_writeback_target",
        );

        if self.efb_depth_resolve_uniform_write_pending {
            let _ = self.submit_pending(queue);
        }

        let uniforms = XfbCopyUniforms {
            src_rect: self.scaled_src_rect(src_x, src_y, width, height),
            dst_size: [encode_w as f32, encode_h as f32],
            gamma: 1.0,
            filter_mode: 0,
        };
        queue.write_buffer(&self.efb_depth_resolve_uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
        self.efb_depth_resolve_uniform_write_pending = true;

        let bytes_per_row = align_up(encode_w as u64 * 4, 256);
        let staging_size = bytes_per_row * encode_h as u64;
        let (staging, staging_capacity) = self.acquire_readback_staging(device, staging_size);

        let mut encoder = self.take_or_create_encoder(device);
        let (writeback_tex, writeback_view) = self.efb_depth_writeback_target.as_ref().unwrap();
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
            rpass.set_pipeline(self.efb_depth_pack_pipelines.for_format(CopyFormat::Z24X8));
            rpass.set_bind_group(0, &self.efb_depth_resolve_bind_group, &[]);
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
        });
    }

    pub(crate) fn ensure_writeback_target(
        device: &wgpu::Device,
        slot: &mut Option<(wgpu::Texture, wgpu::TextureView)>,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        label: &str,
    ) {
        let (target_w, target_h) = slot
            .as_ref()
            .map(|(t, _)| (t.size().width, t.size().height))
            .unwrap_or((0, 0));
        if slot.is_some() && width <= target_w && height <= target_h {
            return;
        }

        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: target_w.max(width).max(64),
                height: target_h.max(height).max(64),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        *slot = Some((tex, view));
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

    fn discard_superseded_writebacks(
        &mut self,
        dest_addr: Address,
        width: u32,
        height: u32,
        copy_format: texture::CopyFormat,
        stride: u32,
    ) {
        let mut i = 0;

        while i < self.pending_writebacks.len() {
            let pending = &self.pending_writebacks[i];

            if pending.dest_addr == dest_addr
                && pending.width == width
                && pending.height == height
                && pending.copy_format == copy_format
                && pending.stride == stride
            {
                let old = self.pending_writebacks.swap_remove(i);
                self.return_readback_staging(old.staging, old.staging_capacity);
            } else {
                i += 1;
            }
        }
    }

    pub(crate) fn return_readback_staging(&mut self, buf: wgpu::Buffer, capacity: u64) {
        const MAX_PER_BUCKET: usize = 8;
        let bucket = self.efb_readback_staging_pool.entry(capacity).or_default();
        if bucket.len() < MAX_PER_BUCKET {
            bucket.push(buf);
        }
    }

    pub(crate) fn drain_pending_writebacks(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ram: &mut gecko::mmio::RamViewMut<'_>,
    ) {
        if self.pending_writebacks.is_empty() {
            return;
        }

        let submission_index = self.submit_pending(queue);

        for pending in &self.pending_writebacks {
            pending
                .staging
                .slice(..pending.staging_size)
                .map_async(wgpu::MapMode::Read, |_| {});
        }

        if let Err(err) = device.poll(wgpu::PollType::Wait {
            submission_index,
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

            let encoded = texture::encode_from_rgba(&rgba, w.width as usize, w.height as usize, w.copy_format);
            let row_bytes = texture::encoded_row_bytes(w.width, w.copy_format);
            let row_count = texture::encoded_row_count(w.height, w.copy_format);
            let dest_stride_bytes = w.stride as usize;

            texture::write_strided_copy_to_ram(ram, w.dest_addr, &encoded, row_bytes, row_count, dest_stride_bytes);

            self.return_readback_staging(w.staging, w.staging_capacity);
        }
    }

    fn ensure_draw_capacity(&mut self, count: usize) {
        if count <= self.draw_uniform_capacity {
            return;
        }
        self.draw_uniform_capacity = count.next_power_of_two();
    }
}

pub(crate) fn grow_capacity(current: u64, needed: u64) -> u64 {
    if needed <= current {
        current
    } else {
        needed.next_power_of_two().max(needed).max(current)
    }
}
