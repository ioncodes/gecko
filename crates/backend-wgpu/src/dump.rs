use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::capture::{self, CapturedFrame};
use crate::{GxRenderer, align_up};

impl GxRenderer {
    pub(crate) fn dump_textures(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, dir: &Path) {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        if let Err(err) = std::fs::create_dir_all(dir) {
            tracing::error!(dir = %dir.display(), ?err, "dump_textures: failed to create dir");
            return;
        }

        let mut saved = 0usize;
        for (cache_id, (fmt, tex, _)) in self.texture_cache.iter() {
            let size = tex.size();
            let width = size.width;
            let height = size.height;
            if width == 0 || height == 0 {
                continue;
            }

            if tex.format() != wgpu::TextureFormat::Rgba8Unorm {
                tracing::warn!(
                    cache_id = *cache_id,
                    wgpu_fmt = ?tex.format(),
                    "dump_textures: skipping non-Rgba8Unorm texture"
                );
                continue;
            }

            let bytes_per_row = align_up(width as u64 * 4, 256);
            let staging_size = bytes_per_row * height as u64;
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("dump_texture_staging"),
                size: staging_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("dump_texture_encoder"),
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
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
            queue.submit([encoder.finish()]);

            let slice = staging.slice(..staging_size);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            if let Err(err) = device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(5)),
            }) {
                tracing::warn!(cache_id = *cache_id, ?err, "dump_textures: device poll failed");
                continue;
            }

            let mut rgba = vec![0u8; (width * height * 4) as usize];
            {
                let mapped = slice.get_mapped_range();
                let row_bytes = (width * 4) as usize;
                let src_row_bytes = bytes_per_row as usize;
                for y in 0..height as usize {
                    let src_row = &mapped[y * src_row_bytes..y * src_row_bytes + row_bytes];
                    let dst_row = &mut rgba[y * row_bytes..y * row_bytes + row_bytes];
                    dst_row.copy_from_slice(src_row);
                }
            }
            staging.unmap();

            let ram_addr = (*cache_id) as u32;
            let variant = (*cache_id >> 32) as u32;
            let filename = format!("{millis}_{ram_addr:08X}_{variant:08X}_{fmt:?}_{width}x{height}.png");
            let path: PathBuf = dir.join(filename);
            capture::save_png_async(path, CapturedFrame { width, height, rgba }, false);
            saved += 1;
        }

        tracing::info!(count = saved, dir = %dir.display(), "dumped textures");
    }
}
