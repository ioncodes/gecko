use crate::dvd::Header;

pub const GCN_BANNER_WIDTH: u32 = 96;
pub const GCN_BANNER_HEIGHT: u32 = 32;

const GCN_PIXELS_OFFSET: usize = 0x20;
const GCN_PIXELS_END: usize = GCN_PIXELS_OFFSET + (GCN_BANNER_WIDTH * GCN_BANNER_HEIGHT * 2) as usize;
const WII_IMET_MAGIC_OFFSET: usize = 0x40;
const WII_U8_OFFSET: usize = 0x600;
const U8_MAGIC: [u8; 4] = [0x55, 0xAA, 0x38, 0x2D];
const TPL_MAGIC: u32 = 0x0020_AF30;

#[derive(Debug, Clone)]
pub struct Banner {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Texture<'a> {
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub pixels: &'a [u8],
    pub palette: Option<Palette<'a>>,
}

pub struct Palette<'a> {
    pub format: u32,
    pub entries: &'a [u8],
}

pub type TextureDecoder = fn(Texture<'_>) -> Option<Vec<u8>>;

pub fn extract(
    header: &Header,
    mut read: impl FnMut(usize, &mut [u8]) -> std::io::Result<()>,
    decode: TextureDecoder,
) -> std::io::Result<Option<Banner>> {
    let fst_offset = header.offset_filesystem.get() as usize;
    let fst_size = header.filesystem_size.get() as usize;
    if fst_size == 0 || fst_offset == 0 {
        return Ok(None);
    }

    let mut fst_buf = vec![0u8; fst_size];
    read(fst_offset, &mut fst_buf)?;

    let shift = if header.is_wii() { 2 } else { 0 };
    let Some((file_offset, mut file_size)) = self::find_file(&fst_buf, "opening.bnr", shift) else {
        return Ok(None);
    };
    if header.is_gc() {
        file_size = file_size.min(GCN_PIXELS_END);
    }

    let mut buf = vec![0u8; file_size];
    read(file_offset, &mut buf)?;

    if buf.len() >= 4 && (&buf[0..4] == b"BNR1" || &buf[0..4] == b"BNR2") {
        return Ok(self::decode_gcn(&buf, decode));
    }

    if header.is_wii() {
        return Ok(self::decode_wii(&buf, decode));
    }

    Ok(None)
}

fn decode_gcn(buf: &[u8], decode: TextureDecoder) -> Option<Banner> {
    let rgba = decode(Texture {
        width: GCN_BANNER_WIDTH,
        height: GCN_BANNER_HEIGHT,
        format: 5,
        pixels: buf.get(GCN_PIXELS_OFFSET..GCN_PIXELS_END)?,
        palette: None,
    })?;

    Some(Banner {
        width: GCN_BANNER_WIDTH,
        height: GCN_BANNER_HEIGHT,
        rgba,
    })
}

fn decode_wii(buf: &[u8], decode: TextureDecoder) -> Option<Banner> {
    if buf.len() < WII_U8_OFFSET + 32 || &buf[WII_IMET_MAGIC_OFFSET..WII_IMET_MAGIC_OFFSET + 4] != b"IMET" {
        tracing::warn!("wii banner: outer layout / IMET magic missing");
        return None;
    }

    let banner_bin = self::read_u8_file(&buf[WII_U8_OFFSET..], "banner.bin")?;
    let inner = self::strip_imd5_and_lz77(banner_bin)?;
    self::decode_largest_tpl(&inner, decode)
}

fn strip_imd5_and_lz77(banner_bin: &[u8]) -> Option<Vec<u8>> {
    if banner_bin.len() < 0x28 || &banner_bin[0..4] != b"IMD5" {
        tracing::warn!("wii banner: missing IMD5 header");
        return None;
    }

    let lz77 = &banner_bin[0x20..];
    if lz77.len() < 8 || &lz77[0..4] != b"LZ77" {
        tracing::warn!("wii banner: missing LZ77 header");
        return None;
    }

    if lz77[4] != 0x10 {
        tracing::warn!(ctrl = lz77[4], "wii banner: unsupported LZ77 variant");
        return None;
    }

    let decompressed_size = (lz77[5] as usize) | ((lz77[6] as usize) << 8) | ((lz77[7] as usize) << 16);
    let inner = self::lz77_decompress(&lz77[8..], decompressed_size);
    if inner.is_none() {
        tracing::warn!("wii banner: LZ77 decompress failed");
    }

    inner
}

fn find_file(fst: &[u8], name: &str, shift: u32) -> Option<(usize, usize)> {
    let count = self::u32_be(fst, 8)? as usize;
    let strings = fst.get(count.checked_mul(12)?..)?;

    for index in 1..count {
        let base = index * 12;
        if fst[base] != 0 {
            continue;
        }

        let name_off = self::u24_be(fst, base + 1)? as usize;
        if self::read_cstr_at(strings, name_off)?.eq_ignore_ascii_case(name) {
            let offset = (self::u32_be(fst, base + 4)? as u64) << shift;
            return Some((usize::try_from(offset).ok()?, self::u32_be(fst, base + 8)? as usize));
        }
    }

    None
}

fn read_u8_file<'a>(archive: &'a [u8], name: &str) -> Option<&'a [u8]> {
    let (nodes_base, node_count, strings) = self::parse_u8_header(archive)?;

    for i in 1..node_count {
        let base = nodes_base + i * 12;
        if archive[base] != 0 {
            continue;
        }

        let name_off = self::u24_be(archive, base + 1)? as usize;
        let file_offset = self::u32_be(archive, base + 4)? as usize;
        let file_size = self::u32_be(archive, base + 8)? as usize;
        let entry_name = self::read_cstr_at(strings, name_off)?;
        if !entry_name.eq_ignore_ascii_case(name) {
            continue;
        }

        return archive.get(file_offset..file_offset.checked_add(file_size)?);
    }

    None
}

fn decode_largest_tpl(archive: &[u8], decode: TextureDecoder) -> Option<Banner> {
    let (nodes_base, node_count, strings) = self::parse_u8_header(archive)?;
    let mut best: Option<Banner> = None;

    for i in 1..node_count {
        let base = nodes_base + i * 12;
        if archive[base] != 0 {
            continue;
        }

        let name_off = self::u24_be(archive, base + 1)? as usize;
        let name = self::read_cstr_at(strings, name_off)?;
        if !name.to_ascii_lowercase().ends_with(".tpl") {
            continue;
        }

        let offset = self::u32_be(archive, base + 4)? as usize;
        let size = self::u32_be(archive, base + 8)? as usize;
        let Some(tpl) = offset.checked_add(size).and_then(|end| archive.get(offset..end)) else {
            continue;
        };

        let Some(count) = self::u32_be(tpl, 4) else { continue };
        if count as usize > tpl.len() / 8 {
            continue;
        }

        for index in 0..count as usize {
            let Some(texture) = self::parse_tpl_texture(tpl, index) else {
                continue;
            };
            if best
                .as_ref()
                .is_some_and(|b| b.width * b.height >= texture.width * texture.height)
            {
                continue;
            }

            let (width, height) = (texture.width, texture.height);
            if let Some(rgba) = decode(texture) {
                best = Some(Banner { width, height, rgba });
            }
        }
    }

    best
}

fn parse_u8_header(archive: &[u8]) -> Option<(usize, usize, &[u8])> {
    if archive.len() < 0x20 || archive[0..4] != U8_MAGIC {
        return None;
    }

    let root_node_offset = self::u32_be(archive, 0x04)? as usize;
    let header_size = self::u32_be(archive, 0x08)? as usize;
    let nodes_base = root_node_offset;
    let node_count = self::u32_be(archive, nodes_base.checked_add(8)?)? as usize;
    if node_count == 0 {
        return None;
    }

    let nodes_end = nodes_base.checked_add(node_count.checked_mul(12)?)?;
    let strings_end = root_node_offset.checked_add(header_size)?;
    if strings_end > archive.len() || nodes_end > strings_end {
        return None;
    }

    let strings = &archive[nodes_end..strings_end];
    Some((nodes_base, node_count, strings))
}

fn parse_tpl_texture(tpl: &[u8], index: usize) -> Option<Texture<'_>> {
    if tpl.len() < 12 {
        return None;
    }

    let magic = self::u32_be(tpl, 0)?;
    if magic != TPL_MAGIC {
        tracing::warn!(magic, "wii banner: bad TPL magic");
        return None;
    }

    let ntex = self::u32_be(tpl, 4)?;
    if index >= ntex as usize {
        return None;
    }

    let table_off = (self::u32_be(tpl, 8)? as usize).checked_add(index.checked_mul(8)?)?;
    let img_hdr_off = self::u32_be(tpl, table_off)? as usize;
    let palette_hdr_off = self::u32_be(tpl, table_off.checked_add(4)?)? as usize;
    if img_hdr_off.checked_add(0x24)? > tpl.len() {
        return None;
    }

    let height = u16::from_be_bytes(tpl[img_hdr_off..img_hdr_off + 2].try_into().ok()?) as usize;
    let width = u16::from_be_bytes(tpl[img_hdr_off + 2..img_hdr_off + 4].try_into().ok()?) as usize;
    let format = self::u32_be(tpl, img_hdr_off + 4)?;
    let data_off = self::u32_be(tpl, img_hdr_off + 8)? as usize;

    if width == 0 || height == 0 || width > 4096 || height > 4096 || data_off >= tpl.len() {
        return None;
    }

    let palette = if matches!(format, 8 | 9 | 10) {
        Some(self::parse_palette(tpl, palette_hdr_off)?)
    } else {
        None
    };

    Some(Texture {
        width: width as u32,
        height: height as u32,
        format,
        pixels: &tpl[data_off..],
        palette,
    })
}

fn parse_palette(tpl: &[u8], offset: usize) -> Option<Palette<'_>> {
    if offset == 0 {
        return None;
    }

    let count = u16::from_be_bytes(tpl.get(offset..offset.checked_add(2)?)?.try_into().ok()?) as usize;
    let format = self::u32_be(tpl, offset.checked_add(4)?)?;
    let start = self::u32_be(tpl, offset.checked_add(8)?)? as usize;
    if count == 0 {
        return None;
    }

    Some(Palette {
        format,
        entries: tpl.get(start..start.checked_add(count * 2)?)?,
    })
}

fn lz77_decompress(src: &[u8], expected_size: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(expected_size);
    let mut i = 0;

    while out.len() < expected_size && i < src.len() {
        let flags = src[i];
        i += 1;

        for bit in 0..8 {
            if out.len() >= expected_size {
                break;
            }

            if flags & (0x80 >> bit) == 0 {
                if i >= src.len() {
                    return None;
                }

                out.push(src[i]);

                i += 1;
            } else {
                if i + 1 >= src.len() {
                    return None;
                }

                let b0 = src[i];
                let b1 = src[i + 1];
                i += 2;

                let length = ((b0 >> 4) as usize) + 3;
                let offset = ((((b0 & 0xF) as usize) << 8) | (b1 as usize)) + 1;
                if offset > out.len() {
                    return None;
                }

                for _ in 0..length {
                    let v = out[out.len() - offset];
                    out.push(v);
                }
            }
        }
    }

    if out.len() != expected_size {
        return None;
    }

    Some(out)
}

fn u32_be(buf: &[u8], off: usize) -> Option<u32> {
    let slice = buf.get(off..off.checked_add(4)?)?;
    Some(u32::from_be_bytes(slice.try_into().ok()?))
}

fn u24_be(buf: &[u8], off: usize) -> Option<u32> {
    let slice = buf.get(off..off.checked_add(3)?)?;
    Some(u32::from_be_bytes([0, slice[0], slice[1], slice[2]]))
}

fn read_cstr_at(buf: &[u8], off: usize) -> Option<&str> {
    let slice = buf.get(off..)?;
    let end = slice.iter().position(|&b| b == 0)?;
    std::str::from_utf8(&slice[..end]).ok()
}
