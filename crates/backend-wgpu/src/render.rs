use crate::{FrameUniforms, GxRenderer};
use crate::{GpuVertex, align_up};
use encase::ShaderType as _;
use gecko::common::Address;
use gecko::flipper::gx::texture;
use gecko::host::{EfbWriteback, XfbPart};

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

        let frame_stride = align_up(
            FrameUniforms::min_size().get(),
            device.limits().min_uniform_buffer_offset_alignment as u64,
        ) as usize;
        let needed_frame_size = (num_draws * frame_stride) as u64;
        if needed_frame_size > self.frame_uniform_buffer.size() {
            self.frame_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gx_frame_uniforms"),
                size: needed_frame_size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.bind_group_cache.clear();
        }

        queue.write_buffer(&self.frame_uniform_buffer, 0, frame_uniform_bytes);
        queue.write_buffer(&self.draw_uniform_buffer, 0, &self.scratch_uniform_bytes);
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.scratch_vertices));
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
        clear: bool,
        clear_color: [f32; 4],
        clear_z: f32,
    ) {
        let width = src_w.min(crate::EFB_WIDTH.saturating_sub(src_x));
        let height = src_h.min(crate::EFB_HEIGHT.saturating_sub(src_y));
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
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("xfb_copy_tmp"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.surface_format,
                usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            (tex, view)
        });

        // Recreate if size changed.
        let existing_size = entry.0.size();
        if existing_size.width != width || existing_size.height != height {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("xfb_copy_tmp"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.surface_format,
                usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let view = tex.create_view(&Default::default());
            *entry = (tex, view);
        }

        let mut encoder = device.create_command_encoder(&Default::default());
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
        queue.submit([encoder.finish()]);

        // Region-scoped EFB clear after copy (if requested).
        if clear {
            self.efb_clear.clear_region(
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
            );
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
            self.xfb_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("xfb_accum"),
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
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.xfb_view = self.xfb_texture.create_view(&Default::default());
        }

        let mut encoder = device.create_command_encoder(&Default::default());

        // Don't clear the XFB: let previous content persist so partial
        // frames show the last valid content instead of a black flash.

        let xfb_size = self.xfb_texture.size();

        for part in parts {
            let Some((tex, _)) = self.xfb_copies.get(&part.id) else {
                tracing::warn!(id = part.id, "present_xfb: XFB copy not found in cache, skipping part");
                continue;
            };
            let src_size = tex.size();
            let width = src_size.width.min(xfb_size.width.saturating_sub(part.offset_x));
            let height = src_size.height.min(xfb_size.height.saturating_sub(part.offset_y));
            if width == 0 || height == 0 {
                tracing::warn!(id = part.id, "present_xfb: zero-area XFB part after clamping, skipping");
                continue;
            }
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

        queue.submit([encoder.finish()]);
        self.xfb_has_content = true;
    }

    /// EFB-to-texture copy: read a region of the resolved EFB back into a
    /// staging buffer, convert from the wgpu surface format to RGBA8,
    /// optional 2x downsample, encode into the requested GX texture format,
    /// and ship the bytes back to the emu thread via the writeback channel.
    ///
    /// `effective_clear` is the already-gated clear flag (see the
    /// `CopyEfbToTexture` arm in `action.rs`); this function does not look
    /// at the per-channel write masks itself.
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
        effective_clear: bool,
        clear_color: [f32; 4],
        clear_z: f32,
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
        let Some(copy_format_enum) = texture::CopyFormat::from_u8(copy_format) else {
            tracing::warn!(
                copy_format = format!("{copy_format:#x}"),
                "efb_to_texture: unsupported copy format, skipping readback"
            );
            if effective_clear {
                self.efb_clear.clear_region(
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
                );
            }
            return;
        };

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
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("efb_to_texture_copy"),
        });
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

        if let Some(tx) = &self.efb_writeback_tx {
            if let Err(err) = tx.try_send(EfbWriteback {
                dest_addr,
                bytes: encoded,
            }) {
                tracing::warn!(?err, "efb_to_texture: writeback channel send failed");
            }
        }

        if effective_clear {
            self.efb_clear.clear_region(
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
