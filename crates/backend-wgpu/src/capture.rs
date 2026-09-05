use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::align_up;

pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureRequest {
    None,
    FullWindow,
    GameOnly,
}

pub struct ScreenshotControl {
    pending: CaptureRequest,
}

impl ScreenshotControl {
    pub fn new() -> Self {
        Self {
            pending: CaptureRequest::None,
        }
    }

    pub fn request(&mut self, req: CaptureRequest) {
        if req != CaptureRequest::None {
            self.pending = req;
        }
    }

    pub fn take_pending(&mut self) -> CaptureRequest {
        std::mem::replace(&mut self.pending, CaptureRequest::None)
    }
}

impl Default for ScreenshotControl {
    fn default() -> Self {
        Self::new()
    }
}

pub fn capture_texture(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Option<CapturedFrame> {
    let size = texture.size();
    let width = size.width;
    let height = size.height;
    if width == 0 || height == 0 {
        return None;
    }

    let format = texture.format();
    let swap = match format {
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => true,
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => false,
        _ => {
            tracing::warn!(?format, "capture_texture: unsupported texture format");
            return None;
        }
    };

    let bytes_per_row = align_up(width as u64 * 4, 256);
    let staging_size = bytes_per_row * height as u64;

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot_staging"),
        size: staging_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("screenshot_encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
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
        tracing::warn!(?err, "capture_texture: device poll failed");
        return None;
    }

    let mut rgba = vec![0u8; (width * height * 4) as usize];
    {
        let mapped = slice.get_mapped_range();
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

    Some(CapturedFrame { width, height, rgba })
}

pub fn save_png_async(path: PathBuf, frame: CapturedFrame, force_opaque: bool) {
    std::thread::Builder::new()
        .name("screenshot-encode".into())
        .spawn(move || match self::write_png(&path, frame, force_opaque) {
            Ok(()) => tracing::info!(path = %path.display(), "saved screenshot"),
            Err(err) => tracing::error!(path = %path.display(), ?err, "screenshot save failed"),
        })
        .ok();
}

pub fn write_png(path: &Path, mut frame: CapturedFrame, force_opaque: bool) -> std::io::Result<()> {
    if force_opaque {
        for px in frame.rgba.chunks_exact_mut(4) {
            px[3] = 255;
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);

    let mut png_writer = encoder.write_header().map_err(std::io::Error::other)?;
    png_writer
        .write_image_data(&frame.rgba)
        .map_err(std::io::Error::other)?;

    Ok(())
}

pub fn timestamped_path(prefix: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    gecko::paths::resolve("screenshots").join(format!("{prefix}_{millis}.png"))
}
