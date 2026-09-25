use gecko::flipper::gx::draw::TextureFormat;
use gecko::flipper::gx::texture::{EncodedTexture, mip_dimensions, raw_data_size};
use image_codec::{ImageReader, RgbaImage};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::{BTreeMap, btree_map};
use std::path::{Path, PathBuf};

const CACHE_BYTES: usize = 256 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

type Error = Box<dyn std::error::Error + Send + Sync>;

type Entry = BTreeMap<u32, PathBuf>;

struct Replacement {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bytes: usize,
    last_used: u64,
}

#[derive(Default)]
pub struct TexturePack {
    entries: BTreeMap<String, Entry>,
    loaded: FxHashMap<String, Replacement>,
    failed: FxHashSet<String>,
    bytes: usize,
    tick: u64,
}

impl TexturePack {
    pub fn discover(root: &Path, game_id: &str) -> Self {
        if game_id.len() != 6 || !game_id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Self::default();
        }

        let exact = root.join(game_id);
        let directory = if exact.is_dir() {
            exact
        } else {
            root.join(&game_id[..3])
        };

        let files = walkdir::WalkDir::new(directory)
            .sort_by_file_name()
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(walkdir::DirEntry::into_path);

        let mut pack = Self::default();
        for path in files {
            let Some(extension) = path.extension().and_then(|s| s.to_str()) else {
                continue;
            };
            if !extension.eq_ignore_ascii_case("png") && !extension.eq_ignore_ascii_case("dds") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !stem.starts_with("tex1_") {
                continue;
            }

            let stem = stem.replace("_arb", "");
            let (name, level) = match stem.rsplit_once("_mip") {
                Some((name, level)) => match level.parse::<u32>() {
                    Ok(level) if level > 0 && level < 32 => (name, level),
                    _ => continue,
                },
                None => (stem.as_str(), 0),
            };
            match pack.entries.entry(name.to_owned()).or_default().entry(level) {
                btree_map::Entry::Vacant(slot) => {
                    slot.insert(path);
                }
                btree_map::Entry::Occupied(_) => {
                    tracing::warn!(path = %path.display(), "duplicate texture replacement; using first path");
                }
            }
        }

        pack.entries.retain(|_, levels| levels.contains_key(&0));
        if !pack.entries.is_empty() {
            tracing::info!(game_id, textures = pack.entries.len(), "indexed Dolphin texture pack");
        }

        pack
    }

    pub(crate) fn lookup(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        format: TextureFormat,
        data: &EncodedTexture,
    ) -> Option<(wgpu::Texture, wgpu::TextureView)> {
        if self.entries.is_empty() {
            return None;
        }

        let names = self::texture_names(width, height, format, data)?;
        let name = names.into_iter().find(|name| self.entries.contains_key(name))?;
        if self.failed.contains(&name) {
            return None;
        }

        self.tick += 1;
        if let Some(cached) = self.loaded.get_mut(&name) {
            cached.last_used = self.tick;
            return Some((cached.texture.clone(), cached.view.clone()));
        }

        let limit = device.limits().max_texture_dimension_2d;
        let levels = match self::load_entry(&self.entries[&name], limit, data.mipmaps_enabled) {
            Ok(levels) => levels,
            Err(err) => {
                tracing::warn!(name, %err, "texture replacement failed; using guest texture");
                self.failed.insert(name);
                return None;
            }
        };

        let replacement = self::upload(device, queue, &name, &levels, self.tick);
        while self.bytes + replacement.bytes > CACHE_BYTES {
            let Some(oldest) = self
                .loaded
                .iter()
                .min_by_key(|(_, r)| r.last_used)
                .map(|(n, _)| n.clone())
            else {
                break;
            };
            self.bytes -= self.loaded.remove(&oldest).unwrap().bytes;
        }

        tracing::debug!(name, "loaded texture replacement");

        let handles = (replacement.texture.clone(), replacement.view.clone());
        self.bytes += replacement.bytes;
        self.loaded.insert(name, replacement);

        Some(handles)
    }
}

fn texture_names(width: u32, height: u32, format: TextureFormat, data: &EncodedTexture) -> Option<[String; 3]> {
    let bytes = data.bytes.get(..raw_data_size(width, height, format))?;
    let hash = twox_hash::xxhash64::Hasher::oneshot(0, bytes);

    let mut palette = String::new();
    if format.is_paletted() {
        let (mut min, mut max) = (usize::MAX, 0);
        let mut index = |value: usize| {
            min = min.min(value);
            max = max.max(value);
        };

        match format {
            TextureFormat::CI4 => {
                for &b in bytes {
                    index((b & 15) as usize);
                    index((b >> 4) as usize);
                }
            }
            TextureFormat::CI8 => {
                for &b in bytes {
                    index(b as usize);
                }
            }
            TextureFormat::CI14 => {
                for b in bytes.chunks_exact(2) {
                    index((u16::from_be_bytes([b[0], b[1]]) & 0x3fff) as usize);
                }
            }
            _ => unreachable!(),
        }

        let used: Vec<u8> = data
            .palette
            .get(min..=max)?
            .iter()
            .flat_map(|p| p.to_be_bytes())
            .collect();
        palette = format!("_{:016x}", twox_hash::xxhash64::Hasher::oneshot(0, &used));
    }

    let base = format!("tex1_{width}x{height}{}", if data.mipmaps_enabled { "_m" } else { "" });
    let fmt = format as u8;
    Some([
        format!("{base}_{hash:016x}{palette}_{fmt}"),
        format!("{base}_{hash:016x}_$_{fmt}"),
        format!("{base}_${palette}_{fmt}"),
    ])
}

fn validate_size(width: u32, height: u32, limit: u32) -> Result<(), Error> {
    if width == 0
        || height == 0
        || width > limit
        || height > limit
        || u64::from(width) * u64::from(height) * 4 > MAX_IMAGE_BYTES
    {
        return Err("replacement dimensions exceed image or device limits".into());
    }
    Ok(())
}

fn load_entry(entry: &Entry, limit: u32, mipmapped: bool) -> Result<Vec<RgbaImage>, Error> {
    let (mut levels, skipped) = self::load_image(&entry[&0], limit)?;
    if !mipmapped {
        levels.truncate(1);
        return Ok(levels);
    }

    let (width, height) = levels[0].dimensions();
    let count = width.max(height).ilog2() + 1;
    for (&source_level, path) in entry.range(1..) {
        let Some(level) = source_level.checked_sub(skipped) else {
            continue;
        };

        let Some(expected) = mip_dimensions(width, height, count).nth(level as usize) else {
            return Err("replacement mip level exceeds texture dimensions".into());
        };

        let (mut images, sidecar_skipped) = self::load_image(path, limit)?;
        if sidecar_skipped != 0 {
            return Err("replacement mip exceeds image limits".into());
        }

        let image = images.swap_remove(0);
        if image.dimensions() != expected {
            return Err("replacement mip dimensions do not match base level".into());
        }

        while levels.len() <= level as usize {
            self::append_mip(&mut levels);
        }

        levels[level as usize] = image;
    }

    while levels.len() < count as usize {
        self::append_mip(&mut levels);
    }

    Ok(levels)
}

fn append_mip(levels: &mut Vec<RgbaImage>) {
    let last = levels.last().unwrap();
    levels.push(image_codec::imageops::resize(
        last,
        (last.width() / 2).max(1),
        (last.height() / 2).max(1),
        image_codec::imageops::FilterType::Triangle,
    ));
}

fn load_image(path: &Path, limit: u32) -> Result<(Vec<RgbaImage>, u32), Error> {
    if std::fs::metadata(path)?.len() > MAX_FILE_BYTES {
        return Err("replacement file too large".into());
    }

    if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("dds")) {
        return self::decode_dds(std::fs::File::open(path)?, limit);
    }

    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image_codec::Limits::default();
    limits.max_image_width = Some(limit);
    limits.max_image_height = Some(limit);
    limits.max_alloc = Some(MAX_IMAGE_BYTES);
    reader.limits(limits);

    let image = reader.decode()?.to_rgba8();
    self::validate_size(image.width(), image.height(), limit)?;

    Ok((vec![image], 0))
}

fn decode_dds(reader: impl std::io::Read, limit: u32) -> Result<(Vec<RgbaImage>, u32), Error> {
    use ddsfile::{D3DFormat as D3d, DxgiFormat as Dxgi};

    let dds = ddsfile::Dds::read(reader)?;
    let (width, height) = (dds.get_width(), dds.get_height());
    if width == 0 || height == 0 {
        return Err("invalid DDS dimensions".into());
    }

    if dds.get_depth() != 1 || dds.get_num_array_layers() != 1 {
        return Err("only 2D DDS replacements are supported".into());
    }

    type Decoder = fn(&[u8], usize, usize, &mut [u32]) -> Result<(), &'static str>;
    let (block_size, decoder): (usize, Decoder) = match (dds.get_d3d_format(), dds.get_dxgi_format()) {
        (Some(D3d::DXT1), _) | (_, Some(Dxgi::BC1_UNorm | Dxgi::BC1_UNorm_sRGB)) => (8, texture2ddecoder::decode_bc1a),
        (Some(D3d::DXT3), _) | (_, Some(Dxgi::BC2_UNorm | Dxgi::BC2_UNorm_sRGB)) => (16, texture2ddecoder::decode_bc2),
        (Some(D3d::DXT5), _) | (_, Some(Dxgi::BC3_UNorm | Dxgi::BC3_UNorm_sRGB)) => (16, texture2ddecoder::decode_bc3),
        (_, Some(Dxgi::BC7_UNorm | Dxgi::BC7_UNorm_sRGB)) => (16, texture2ddecoder::decode_bc7),
        _ => return Err("unsupported DDS format (expected BC1/DXT1, BC2/DXT3, BC3/DXT5 or BC7)".into()),
    };

    let count = dds.get_num_mipmap_levels();
    if count == 0 || count > width.max(height).ilog2() + 1 {
        return Err("invalid DDS mip count".into());
    }

    let mut offset = 0;
    let mut levels = Vec::new();
    let mut skipped = 0;
    for (w, h) in mip_dimensions(width, height, count) {
        let len = w.div_ceil(4) as usize * h.div_ceil(4) as usize * block_size;
        let source = dds.data.get(offset..offset + len).ok_or("truncated DDS mip data")?;

        offset += len;
        if self::validate_size(w, h, limit).is_err() {
            skipped += 1;
            continue;
        }

        let mut pixels = vec![0; w as usize * h as usize];
        decoder(source, w as usize, h as usize, &mut pixels)?;

        let rgba = pixels
            .into_iter()
            .flat_map(|p| {
                let [b, g, r, a] = p.to_le_bytes();
                [r, g, b, a]
            })
            .collect();
        levels.push(RgbaImage::from_raw(w, h, rgba).ok_or("invalid DDS dimensions")?);
    }

    if levels.is_empty() {
        return Err("DDS has no mip within image or device limits".into());
    }

    Ok((levels, skipped))
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, name: &str, levels: &[RgbaImage], tick: u64) -> Replacement {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(name),
        size: wgpu::Extent3d {
            width: levels[0].width(),
            height: levels[0].height(),
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    for (level, image) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                mip_level: level as u32,
                ..texture.as_image_copy()
            },
            image.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width() * 4),
                rows_per_image: Some(image.height()),
            },
            wgpu::Extent3d {
                width: image.width(),
                height: image.height(),
                depth_or_array_layers: 1,
            },
        );
    }

    let view = texture.create_view(&Default::default());

    Replacement {
        texture,
        view,
        bytes: levels.iter().map(|image| image.as_raw().len()).sum(),
        last_used: tick,
    }
}
