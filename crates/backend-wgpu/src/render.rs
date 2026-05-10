#[cfg(feature = "efb-writeback")]
use crate::align_up;
use crate::{GpuVertex, GxRenderer};
use gecko::common::Address;
#[cfg(feature = "efb-writeback")]
use gecko::flipper::gx::texture;
#[cfg(feature = "efb-writeback")]
use gecko::host::EfbWriteback;
use gecko::host::XfbPart;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct XfbCopyUniforms {
    src_rect: [f32; 4],
    dst_size: [f32; 2],
    gamma: f32,
    filter_mode: u32,
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

        let needs_shader_copy = dst_h != height || (gamma - 1.0).abs() > f32::EPSILON;
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
        queue.submit([encoder.finish()]);

        // Region-scoped EFB clear after copy (if requested).
        if clear {
            self.efb_clear.clear_region_masked(
                device,
                queue,
                &self.efb_msaa_view,
                &self.efb_view,
                &self.efb_depth_view,
                crate::EFB_WIDTH,
                crate::EFB_HEIGHT,
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

    /// Blits the resolved EFB region into a GPU texture keyed by `dest_addr` for later texture binds to sample. Color only.
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
    ) {
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

        // Send any prior cached entry back to the right pool so it can be reused.
        if let Some((old_tex, old_view)) = self.efb_copy_cache.remove(&dest_addr) {
            self.return_to_pool(old_tex, old_view);
        }
        self.bind_group_cache
            .retain(|key, _| !key.tex_keys.iter().any(|k| k.map(|t| t.ram_addr) == Some(dest_addr)));

        let (tex, view) = self
            .efb_copy_pool
            .get_mut(&(dst_w, dst_h))
            .and_then(Vec::pop)
            .unwrap_or_else(|| {
                let label = format!("efb_copy addr={dest_addr:#010x} size={dst_w}x{dst_h}");
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
                        | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = tex.create_view(&Default::default());
                (tex, view)
            });

        let uniforms = XfbCopyUniforms {
            src_rect: [src_x as f32, src_y as f32, width as f32, height as f32],
            dst_size: [dst_w as f32, dst_h as f32],
            gamma: 1.0,
            filter_mode: 0,
        };
        queue.write_buffer(&self.xfb_copy_uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("efb_copy_bg"),
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
            "CopyEfbToTexture addr={dest_addr:#010x} src=({src_x},{src_y} {width}x{height}) dst={dst_w}x{dst_h}"
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("efb_copy_encoder"),
        });
        encoder.push_debug_group(&group_label);
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("efb_copy"),
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
            rpass.set_pipeline(&self.efb_copy_pipeline);
            rpass.set_bind_group(0, &bind_group, &[]);
            rpass.insert_debug_marker("EFB copy: resolved color to texture cache");
            rpass.draw(0..3, 0..1);
        }
        encoder.pop_debug_group();
        queue.submit([encoder.finish()]);

        self.efb_copy_cache.insert(dest_addr, (tex, view));
    }

    /// Sends an evicted efb_copy_cache entry back to the correct size-keyed pool based on its format.
    pub(crate) fn return_to_pool(&mut self, tex: wgpu::Texture, view: wgpu::TextureView) {
        let size = tex.size();
        let key = (size.width, size.height);
        match tex.format() {
            wgpu::TextureFormat::R16Float => self.efb_depth_pool.entry(key).or_default().push((tex, view)),
            _ => self.efb_copy_pool.entry(key).or_default().push((tex, view)),
        }
    }

    /// Resolves the 4x MSAA `efb_depth_view` into a single-sample R32Float texture keyed by `dest_addr`.
    pub(crate) fn cache_efb_copy_depth(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dest_addr: Address,
        src_x: u32,
        src_y: u32,
        src_w: u32,
        src_h: u32,
        half: bool,
    ) {
        let width = src_w.min(crate::EFB_WIDTH.saturating_sub(src_x));
        let height = src_h.min(crate::EFB_HEIGHT.saturating_sub(src_y));
        if width == 0 || height == 0 {
            tracing::warn!(
                src_x,
                src_y,
                src_w,
                src_h,
                "efb_depth_cache: zero-area region after clamping, skipping"
            );
            return;
        }
        let divisor = if half { 2 } else { 1 };
        let dst_w = (width / divisor).max(1);
        let dst_h = (height / divisor).max(1);

        if let Some((old_tex, old_view)) = self.efb_copy_cache.remove(&dest_addr) {
            self.return_to_pool(old_tex, old_view);
        }
        self.bind_group_cache
            .retain(|key, _| !key.tex_keys.iter().any(|k| k.map(|t| t.ram_addr) == Some(dest_addr)));

        let (tex, view) = self
            .efb_depth_pool
            .get_mut(&(dst_w, dst_h))
            .and_then(Vec::pop)
            .unwrap_or_else(|| {
                let label = format!("efb_depth addr={dest_addr:#010x} size={dst_w}x{dst_h}");
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
                    format: wgpu::TextureFormat::R16Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                });
                let view = tex.create_view(&Default::default());
                (tex, view)
            });

        let uniforms = XfbCopyUniforms {
            src_rect: [src_x as f32, src_y as f32, width as f32, height as f32],
            dst_size: [dst_w as f32, dst_h as f32],
            gamma: 1.0,
            filter_mode: 0,
        };
        queue.write_buffer(&self.efb_depth_resolve_uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("efb_depth_resolve_bg"),
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

        let group_label = format!(
            "CopyEfbDepthToTexture addr={dest_addr:#010x} src=({src_x},{src_y} {width}x{height}) dst={dst_w}x{dst_h}"
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("efb_depth_resolve_encoder"),
        });
        encoder.push_debug_group(&group_label);
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("efb_depth_resolve"),
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
            rpass.set_pipeline(&self.efb_depth_resolve_pipeline);
            rpass.set_bind_group(0, &bind_group, &[]);
            rpass.insert_debug_marker("EFB copy: resolve depth to texture cache");
            rpass.draw(0..3, 0..1);
        }
        encoder.pop_debug_group();
        queue.submit([encoder.finish()]);

        self.efb_copy_cache.insert(dest_addr, (tex, view));
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
        queue.submit([encoder.finish()]);
        self.xfb_has_content = true;
    }

    /// EFB-to-texture copy: read a region of the resolved EFB back into a
    /// staging buffer, convert from the wgpu surface format to RGBA8,
    /// optional 2x downsample, encode into the requested GX texture format,
    /// and ship the bytes back to the emu thread via the writeback channel.
    ///
    /// The clear flag and per-channel update masks are applied after the
    /// copy, matching GX copy-clear ordering.
    ///
    /// Only compiled with `efb-writeback`. The default no-feature path
    /// handles CopyEfbToTexture inline in `action.rs` with just a clear.
    #[cfg(feature = "efb-writeback")]
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
        clear: bool,
        clear_color: [f32; 4],
        clear_z: f32,
        color_update: bool,
        alpha_update: bool,
        z_update: bool,
        alpha_supported: bool,
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

        // Early-out for formats we don't encode: skip the expensive readback
        // but still honor the clear.
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
            if clear {
                self.efb_clear.clear_region_masked(
                    device,
                    queue,
                    &self.efb_msaa_view,
                    &self.efb_view,
                    &self.efb_depth_view,
                    crate::EFB_WIDTH,
                    crate::EFB_HEIGHT,
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
            return;
        };

        if depth_copy {
            tracing::warn!(
                copy_format = format!("{copy_format:#x}"),
                "efb_to_texture: depth readback is not implemented yet, skipping readback"
            );
            if clear {
                self.efb_clear.clear_region_masked(
                    device,
                    queue,
                    &self.efb_msaa_view,
                    &self.efb_view,
                    &self.efb_depth_view,
                    crate::EFB_WIDTH,
                    crate::EFB_HEIGHT,
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
            return;
        }

        // wgpu requires 256-byte row alignment for texture<->buffer copies.
        let bytes_per_row = align_up(width as u64 * 4, 256);
        let staging_size = bytes_per_row * height as u64;

        // Grow staging buffer on demand.
        if self.efb_readback_staging.is_none() || self.efb_readback_capacity < staging_size {
            let new_cap = staging_size.next_power_of_two().max(4096);
            self.efb_readback_staging = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("efb_readback_staging"),
                size: new_cap,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
            self.efb_readback_capacity = new_cap;
        }
        let staging = self.efb_readback_staging.as_ref().unwrap();

        // Submit the EFB -> staging copy.
        let group_label = format!(
            "CopyEfbToTexture addr={dest_addr:#010x} src=({src_x},{src_y} {width}x{height}) fmt={copy_format_enum:?} mip={mipmap} stride={stride} depth={depth_copy}"
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("efb_to_texture_copy_encoder"),
        });
        encoder.push_debug_group(&group_label);
        let copy_marker = format!(
            "EFB readback copy: bytes_per_row={} staging_size={}",
            bytes_per_row, staging_size
        );
        encoder.insert_debug_marker(&copy_marker);
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
                buffer: staging,
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
        queue.submit([encoder.finish()]);

        // Map and wait. This stalls the renderer worker (not the emu
        // thread). Hello zayd, this mirrors beanwii's synchronous glReadPixels I think?
        let slice = staging.slice(..staging_size);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        if let Err(err) = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(5)),
        }) {
            tracing::warn!(?err, "efb_to_texture: device poll failed");
            return;
        }

        // Extract RGBA8, converting from BGRA if the surface format
        // requires it, and stripping wgpu's row padding.
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        {
            let mapped = slice.get_mapped_range();
            let swap = matches!(
                self.surface_format,
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
            );
            let row_bytes = (width * 4) as usize;
            let src_row_bytes = bytes_per_row as usize;
            for y in 0..height as usize {
                let src_row = &mapped[y * src_row_bytes..y * src_row_bytes + row_bytes];
                let dst_row = &mut rgba[y * row_bytes..y * row_bytes + row_bytes];
                if swap {
                    for i in 0..width as usize {
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
        staging.unmap();

        // Optional 2x box-filter downsample.
        let (encode_w, encode_h, encode_src) = if mipmap {
            let down = texture::downsample_box_2x(&rgba, width, height);
            (width / 2, height / 2, down)
        } else {
            (width, height, rgba)
        };

        // Encode and ship back.
        let encoded = texture::encode_from_rgba(&encode_src, encode_w as usize, encode_h as usize, copy_format_enum);
        let row_bytes = texture::encoded_row_bytes(encode_w, copy_format_enum);
        let row_count = texture::encoded_row_count(encode_h, copy_format_enum);
        let dest_stride_bytes = stride as usize;

        if let Some(tx) = &self.efb_writeback_tx {
            if let Err(err) = tx.try_send(EfbWriteback {
                dest_addr,
                bytes: encoded,
                row_bytes,
                row_count,
                dest_stride_bytes,
            }) {
                tracing::warn!(?err, "efb_to_texture: writeback channel send failed");
            }
        }

        if clear {
            self.efb_clear.clear_region_masked(
                device,
                queue,
                &self.efb_msaa_view,
                &self.efb_view,
                &self.efb_depth_view,
                crate::EFB_WIDTH,
                crate::EFB_HEIGHT,
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
