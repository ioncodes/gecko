use super::draw::{TextureDescriptor, TextureFormat};

/// Decode a GX-format texture from RAM into RGBA8 pixels.
pub fn decode_to_rgba(ram: &[u8], desc: &TextureDescriptor) -> Vec<u8> {
    let w = desc.width as usize;
    let h = desc.height as usize;

    let mut rgba = vec![0u8; w * h * 4];
    match desc.format {
        TextureFormat::I4 => decode_i4(ram, desc, &mut rgba, w, h),
        TextureFormat::I8 => decode_i8(ram, desc, &mut rgba, w, h),
        TextureFormat::IA4 => decode_ia4(ram, desc, &mut rgba, w, h),
        TextureFormat::IA8 => decode_ia8(ram, desc, &mut rgba, w, h),
        TextureFormat::RGB565 => decode_rgb565(ram, desc, &mut rgba, w, h),
        TextureFormat::RGB5A3 => decode_rgb5a3(ram, desc, &mut rgba, w, h),
        TextureFormat::RGBA8 => decode_rgba8(ram, desc, &mut rgba, w, h),
        TextureFormat::CMPR => decode_cmpr(ram, desc, &mut rgba, w, h),
        _ => tracing::error!(?desc.format, "unsupported texture format"),
    }

    rgba
}

#[derive(Clone, Copy, Debug)]
pub struct BlockDims {
    pub tile_w: u32,
    pub tile_h: u32,
    pub bytes_per_tile: u32,
}

impl BlockDims {
    pub const fn bytes_for(self, w: u32, h: u32) -> usize {
        (w.div_ceil(self.tile_w) * h.div_ceil(self.tile_h) * self.bytes_per_tile) as usize
    }
}

pub const fn block_dims(format: TextureFormat) -> BlockDims {
    let (tw, th, bb) = match format {
        TextureFormat::I4 => (8, 8, 32),
        TextureFormat::I8 => (8, 4, 32),
        TextureFormat::IA4 => (8, 4, 32),
        TextureFormat::IA8 => (4, 4, 32),
        TextureFormat::RGB565 => (4, 4, 32),
        TextureFormat::RGB5A3 => (4, 4, 32),
        TextureFormat::RGBA8 => (4, 4, 64),
        TextureFormat::CMPR => (8, 8, 32),
        TextureFormat::CI4 => (8, 8, 32),
        TextureFormat::CI8 => (8, 4, 32),
        TextureFormat::CI14 => (4, 4, 32),
    };
    BlockDims {
        tile_w: tw,
        tile_h: th,
        bytes_per_tile: bb,
    }
}

/// Compute the number of raw bytes a GX texture occupies in RAM.
#[inline(always)]
pub fn raw_data_size(width: u32, height: u32, format: TextureFormat) -> usize {
    block_dims(format).bytes_for(width, height)
}

fn decode_i4(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], w: usize, h: usize) {
    const BW: usize = 8;
    const BH: usize = 8;
    const BB: usize = 32;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);
    if desc.ram_addr + bcx * bcy * BB > ram.len() {
        tracing::warn!(addr = desc.ram_addr, w, h, "decode_i4: texture OOB, skipping");
        return;
    }

    let src = ram.as_ptr();
    let dst = rgba.as_mut_ptr();

    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = desc.ram_addr + (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let byte = *src.add(blk + (ty * BW + tx) / 2);
                        let nibble = if tx & 1 == 0 { byte >> 4 } else { byte & 0x0F };
                        let i = expand_to_8::<4>(nibble);
                        std::ptr::write(
                            dst.add(((base_y + ty) * w + base_x + tx) * 4).cast::<[u8; 4]>(),
                            [i, i, i, i],
                        );
                    }
                }
            }
        }
    }
}

fn decode_i8(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], w: usize, h: usize) {
    const BW: usize = 8;
    const BH: usize = 4;
    const BB: usize = 32;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);
    if desc.ram_addr + bcx * bcy * BB > ram.len() {
        tracing::warn!(addr = desc.ram_addr, w, h, "decode_i8: texture OOB, skipping");
        return;
    }

    let src = ram.as_ptr();
    let dst = rgba.as_mut_ptr();

    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = desc.ram_addr + (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let i = *src.add(blk + ty * BW + tx);
                        std::ptr::write(
                            dst.add(((base_y + ty) * w + base_x + tx) * 4).cast::<[u8; 4]>(),
                            [i, i, i, i],
                        );
                    }
                }
            }
        }
    }
}

fn decode_ia4(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], w: usize, h: usize) {
    const BW: usize = 8;
    const BH: usize = 4;
    const BB: usize = 32;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);
    if desc.ram_addr + bcx * bcy * BB > ram.len() {
        tracing::warn!(addr = desc.ram_addr, w, h, "decode_ia4: texture OOB, skipping");
        return;
    }

    let src = ram.as_ptr();
    let dst = rgba.as_mut_ptr();

    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = desc.ram_addr + (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let packed = *src.add(blk + ty * BW + tx);
                        let a = expand_to_8::<4>(packed >> 4);
                        let i = expand_to_8::<4>(packed & 0x0F);
                        std::ptr::write(
                            dst.add(((base_y + ty) * w + base_x + tx) * 4).cast::<[u8; 4]>(),
                            [i, i, i, a],
                        );
                    }
                }
            }
        }
    }
}

fn decode_ia8(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], w: usize, h: usize) {
    const BW: usize = 4;
    const BH: usize = 4;
    const BB: usize = 32;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);

    if desc.ram_addr + bcx * bcy * BB > ram.len() {
        tracing::warn!(addr = desc.ram_addr, w, h, "decode_ia8: texture OOB, skipping");
        return;
    }

    let src = ram.as_ptr();
    let dst = rgba.as_mut_ptr();

    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = desc.ram_addr + (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let off = blk + (ty * BW + tx) * 2;
                        let a = *src.add(off);
                        let i = *src.add(off + 1);
                        std::ptr::write(
                            dst.add(((base_y + ty) * w + base_x + tx) * 4).cast::<[u8; 4]>(),
                            [i, i, i, a],
                        );
                    }
                }
            }
        }
    }
}

fn decode_rgb565(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], w: usize, h: usize) {
    const BW: usize = 4;
    const BH: usize = 4;
    const BB: usize = 32;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);
    if desc.ram_addr + bcx * bcy * BB > ram.len() {
        tracing::warn!(addr = desc.ram_addr, w, h, "decode_rgb565: texture OOB, skipping");
        return;
    }

    let src = ram.as_ptr();
    let dst = rgba.as_mut_ptr();

    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = desc.ram_addr + (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let packed = u16::from_be_bytes([
                            *src.add(blk + (ty * BW + tx) * 2),
                            *src.add(blk + (ty * BW + tx) * 2 + 1),
                        ]);
                        let pixel = self::rgb565_to_rgba(packed);
                        std::ptr::write(dst.add(((base_y + ty) * w + base_x + tx) * 4).cast::<[u8; 4]>(), pixel);
                    }
                }
            }
        }
    }
}

fn decode_rgb5a3(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], w: usize, h: usize) {
    const BW: usize = 4;
    const BH: usize = 4;
    const BB: usize = 32;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);
    if desc.ram_addr + bcx * bcy * BB > ram.len() {
        tracing::warn!(addr = desc.ram_addr, w, h, "decode_rgb5a3: texture OOB, skipping");
        return;
    }

    let src = ram.as_ptr();
    let dst = rgba.as_mut_ptr();

    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = desc.ram_addr + (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let packed = u16::from_be_bytes([
                            *src.add(blk + (ty * BW + tx) * 2),
                            *src.add(blk + (ty * BW + tx) * 2 + 1),
                        ]);
                        let pixel = if packed & 0x8000 != 0 {
                            let r = expand_to_8::<5>(((packed >> 10) & 0x1F) as u8);
                            let g = expand_to_8::<5>(((packed >> 5) & 0x1F) as u8);
                            let b = expand_to_8::<5>((packed & 0x1F) as u8);
                            [r, g, b, 255]
                        } else {
                            let a = expand_to_8::<3>(((packed >> 12) & 0x7) as u8);
                            let r = expand_to_8::<4>(((packed >> 8) & 0xF) as u8);
                            let g = expand_to_8::<4>(((packed >> 4) & 0xF) as u8);
                            let b = expand_to_8::<4>((packed & 0xF) as u8);
                            [r, g, b, a]
                        };
                        std::ptr::write(dst.add(((base_y + ty) * w + base_x + tx) * 4).cast::<[u8; 4]>(), pixel);
                    }
                }
            }
        }
    }
}

fn decode_rgba8(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], w: usize, h: usize) {
    const BW: usize = 4;
    const BH: usize = 4;
    const BB: usize = 64;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);
    if desc.ram_addr + bcx * bcy * BB > ram.len() {
        tracing::warn!(addr = desc.ram_addr, w, h, "decode_rgba8: texture OOB, skipping");
        return;
    }

    let src = ram.as_ptr();
    let dst = rgba.as_mut_ptr();

    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = desc.ram_addr + (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let ti = ty * BW + tx;
                        let ar = blk + ti * 2;
                        let gb = blk + 32 + ti * 2;
                        let a = *src.add(ar);
                        let r = *src.add(ar + 1);
                        let g = *src.add(gb);
                        let b = *src.add(gb + 1);
                        std::ptr::write(
                            dst.add(((base_y + ty) * w + base_x + tx) * 4).cast::<[u8; 4]>(),
                            [r, g, b, a],
                        );
                    }
                }
            }
        }
    }
}

fn decode_cmpr(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], w: usize, h: usize) {
    const MW: usize = 8;
    const MH: usize = 8;
    const MB: usize = 32;
    const SB: usize = 8;

    let bcx = w.div_ceil(MW);
    let bcy = h.div_ceil(MH);
    if desc.ram_addr + bcx * bcy * MB > ram.len() {
        tracing::warn!(addr = desc.ram_addr, w, h, "decode_cmpr: texture OOB, skipping");
        return;
    }

    let src = ram.as_ptr();
    let dst = rgba.as_mut_ptr();

    for by in 0..bcy {
        let macro_y = by * MH;
        for bx in 0..bcx {
            let macro_x = bx * MW;
            let macro_off = desc.ram_addr + (by * bcx + bx) * MB;

            for si in 0..4usize {
                let sub_off = macro_off + si * SB;
                let sub_x = macro_x + (si & 1) * 4;
                let sub_y = macro_y + (si >> 1) * 4;
                if sub_x >= w || sub_y >= h {
                    continue;
                }
                let tw = 4usize.min(w - sub_x);
                let th = 4usize.min(h - sub_y);

                unsafe {
                    let c0 = u16::from_be_bytes([*src.add(sub_off), *src.add(sub_off + 1)]);
                    let c1 = u16::from_be_bytes([*src.add(sub_off + 2), *src.add(sub_off + 3)]);
                    let palette = self::build_dxt1_palette(c0, c1);

                    for row in 0..th {
                        let idx = *src.add(sub_off + 4 + row);
                        for col in 0..tw {
                            let pi = ((idx >> ((3 - col) * 2)) & 0x03) as usize;
                            std::ptr::write(
                                dst.add(((sub_y + row) * w + sub_x + col) * 4).cast::<[u8; 4]>(),
                                palette[pi],
                            );
                        }
                    }
                }
            }
        }
    }
}

#[inline(always)]
fn build_dxt1_palette(c0: u16, c1: u16) -> [[u8; 4]; 4] {
    let [r0, g0, b0, _] = self::rgb565_to_rgba(c0);
    let [r1, g1, b1, _] = self::rgb565_to_rgba(c1);

    let mut p = [[0u8; 4]; 4];
    p[0] = [r0, g0, b0, 255];
    p[1] = [r1, g1, b1, 255];

    if c0 > c1 {
        p[2] = [
            ((r0 as u16 * 2 + r1 as u16) / 3) as u8,
            ((g0 as u16 * 2 + g1 as u16) / 3) as u8,
            ((b0 as u16 * 2 + b1 as u16) / 3) as u8,
            255,
        ];
        p[3] = [
            ((r0 as u16 + r1 as u16 * 2) / 3) as u8,
            ((g0 as u16 + g1 as u16 * 2) / 3) as u8,
            ((b0 as u16 + b1 as u16 * 2) / 3) as u8,
            255,
        ];
    } else {
        let avg_r = ((r0 as u16 + r1 as u16) / 2) as u8;
        let avg_g = ((g0 as u16 + g1 as u16) / 2) as u8;
        let avg_b = ((b0 as u16 + b1 as u16) / 2) as u8;
        p[2] = [avg_r, avg_g, avg_b, 255];
        p[3] = [avg_r, avg_g, avg_b, 0];
    }

    p
}

// i have cancer
#[inline(always)]
const fn expand_to_8<const BITS: u32>(v: u8) -> u8 {
    let mut result = v << (8 - BITS);
    let mut pos = 8 - BITS;
    while pos > 0 {
        if pos >= BITS {
            pos -= BITS;
            result |= v << pos;
        } else {
            result |= v >> (BITS - pos);
            break;
        }
    }
    result
}

#[inline(always)]
fn rgb565_to_rgba(packed: u16) -> [u8; 4] {
    let r = ((packed >> 11) & 0x1F) as u8;
    let g = ((packed >> 5) & 0x3F) as u8;
    let b = (packed & 0x1F) as u8;
    [expand_to_8::<5>(r), expand_to_8::<6>(g), expand_to_8::<5>(b), 255]
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CopyFormat {
    I8 = 0x1,
    RGB565 = 0x4,
    RGB5A3 = 0x5,
    RGBA8 = 0x6,
    A8 = 0x7,
    R8 = 0x8,
    RG8 = 0xB,
}

impl CopyFormat {
    pub fn from_u8(code: u8) -> Option<Self> {
        Some(match code {
            0x1 => Self::I8,
            0x4 => Self::RGB565,
            0x5 => Self::RGB5A3,
            0x6 => Self::RGBA8,
            0x7 => Self::A8,
            0x8 => Self::R8,
            0xB => Self::RG8,
            _ => return None,
        })
    }
}

pub fn encoded_size(w: u32, h: u32, format: CopyFormat) -> usize {
    let dims = match format {
        CopyFormat::I8 => block_dims(TextureFormat::I8),
        CopyFormat::RGB565 => block_dims(TextureFormat::RGB565),
        CopyFormat::RGB5A3 => block_dims(TextureFormat::RGB5A3),
        CopyFormat::RGBA8 => block_dims(TextureFormat::RGBA8),
        CopyFormat::A8 | CopyFormat::R8 => BlockDims {
            tile_w: 8,
            tile_h: 4,
            bytes_per_tile: 32,
        },
        CopyFormat::RG8 => BlockDims {
            tile_w: 4,
            tile_h: 4,
            bytes_per_tile: 32,
        },
    };
    dims.bytes_for(w, h)
}

// ref downsample_rgba_buffer_by_2 @ beanwii, zayd is smarter than me

pub fn downsample_box_2x(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let nw = (w / 2) as usize;
    let nh = (h / 2) as usize;
    let sw = w as usize;
    let mut out = vec![0u8; nw * nh * 4];

    for y in 0..nh {
        for x in 0..nw {
            let sx = x * 2;
            let sy = y * 2;
            let s0 = (sy * sw + sx) * 4;
            let s1 = (sy * sw + sx + 1) * 4;
            let s2 = ((sy + 1) * sw + sx) * 4;
            let s3 = ((sy + 1) * sw + sx + 1) * 4;
            let d = (y * nw + x) * 4;
            for c in 0..4 {
                out[d + c] =
                    ((src[s0 + c] as u16 + src[s1 + c] as u16 + src[s2 + c] as u16 + src[s3 + c] as u16) / 4) as u8;
            }
        }
    }
    out
}

/// Encode a linear RGBA8 buffer into a freshly-allocated tiled GX copy
/// format. Mirror of [`decode_to_rgba`] for the EFB-to-texture direction.
pub fn encode_from_rgba(rgba: &[u8], w: usize, h: usize, format: CopyFormat) -> Vec<u8> {
    let size = encoded_size(w as u32, h as u32, format);
    let mut out = vec![0u8; size];
    match format {
        CopyFormat::I8 => encode_i8(rgba, &mut out, w, h),
        CopyFormat::A8 => encode_a8(rgba, &mut out, w, h),
        CopyFormat::R8 => encode_r8(rgba, &mut out, w, h),
        CopyFormat::RG8 => encode_rg8(rgba, &mut out, w, h),
        CopyFormat::RGB565 => encode_rgb565(rgba, &mut out, w, h),
        CopyFormat::RGB5A3 => encode_rgb5a3(rgba, &mut out, w, h),
        CopyFormat::RGBA8 => encode_rgba8(rgba, &mut out, w, h),
    }
    out
}

#[inline(always)]
fn luminance(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8
}

fn encode_i8(rgba: &[u8], out: &mut [u8], w: usize, h: usize) {
    const BW: usize = 8;
    const BH: usize = 4;
    const BB: usize = 32;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);
    if bcx * bcy * BB > out.len() || rgba.len() < w * h * 4 {
        tracing::warn!(w, h, "encode_i8: buffer OOB, skipping");
        return;
    }

    let src = rgba.as_ptr();
    let dst = out.as_mut_ptr();

    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let s = ((base_y + ty) * w + base_x + tx) * 4;
                        let r = *src.add(s);
                        let g = *src.add(s + 1);
                        let b = *src.add(s + 2);
                        *dst.add(blk + ty * BW + tx) = luminance(r, g, b);
                    }
                }
            }
        }
    }
}

fn encode_a8(rgba: &[u8], out: &mut [u8], w: usize, h: usize) {
    const BW: usize = 8;
    const BH: usize = 4;
    const BB: usize = 32;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);
    if bcx * bcy * BB > out.len() || rgba.len() < w * h * 4 {
        tracing::warn!(w, h, "encode_a8: buffer OOB, skipping");
        return;
    }

    let src = rgba.as_ptr();
    let dst = out.as_mut_ptr();

    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let s = ((base_y + ty) * w + base_x + tx) * 4;
                        *dst.add(blk + ty * BW + tx) = *src.add(s + 3);
                    }
                }
            }
        }
    }
}

fn encode_r8(rgba: &[u8], out: &mut [u8], w: usize, h: usize) {
    const BW: usize = 8;
    const BH: usize = 4;
    const BB: usize = 32;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);
    if bcx * bcy * BB > out.len() || rgba.len() < w * h * 4 {
        tracing::warn!(w, h, "encode_r8: buffer OOB, skipping");
        return;
    }

    let src = rgba.as_ptr();
    let dst = out.as_mut_ptr();

    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let s = ((base_y + ty) * w + base_x + tx) * 4;
                        *dst.add(blk + ty * BW + tx) = *src.add(s);
                    }
                }
            }
        }
    }
}

fn encode_rg8(rgba: &[u8], out: &mut [u8], w: usize, h: usize) {
    const BW: usize = 4;
    const BH: usize = 4;
    const BB: usize = 32;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);
    if bcx * bcy * BB > out.len() || rgba.len() < w * h * 4 {
        tracing::warn!(w, h, "encode_rg8: buffer OOB, skipping");
        return;
    }

    let src = rgba.as_ptr();
    let dst = out.as_mut_ptr();

    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let s = ((base_y + ty) * w + base_x + tx) * 4;
                        let off = blk + (ty * BW + tx) * 2;
                        std::ptr::write(dst.add(off).cast::<[u8; 2]>(), [*src.add(s), *src.add(s + 1)]);
                    }
                }
            }
        }
    }
}

fn encode_rgb565(rgba: &[u8], out: &mut [u8], w: usize, h: usize) {
    const BW: usize = 4;
    const BH: usize = 4;
    const BB: usize = 32;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);
    if bcx * bcy * BB > out.len() || rgba.len() < w * h * 4 {
        tracing::warn!(w, h, "encode_rgb565: buffer OOB, skipping");
        return;
    }

    let src = rgba.as_ptr();
    let dst = out.as_mut_ptr();

    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let s = ((base_y + ty) * w + base_x + tx) * 4;
                        let r = (*src.add(s) >> 3) as u16;
                        let g = (*src.add(s + 1) >> 2) as u16;
                        let b = (*src.add(s + 2) >> 3) as u16;
                        let pixel = (r << 11) | (g << 5) | b;
                        let off = blk + (ty * BW + tx) * 2;
                        std::ptr::write(
                            dst.add(off).cast::<[u8; 2]>(),
                            [(pixel >> 8) as u8, (pixel & 0xFF) as u8],
                        );
                    }
                }
            }
        }
    }
}

fn encode_rgb5a3(rgba: &[u8], out: &mut [u8], w: usize, h: usize) {
    const BW: usize = 4;
    const BH: usize = 4;
    const BB: usize = 32;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);
    if bcx * bcy * BB > out.len() || rgba.len() < w * h * 4 {
        tracing::warn!(w, h, "encode_rgb5a3: buffer OOB, skipping");
        return;
    }

    let src = rgba.as_ptr();
    let dst = out.as_mut_ptr();

    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let s = ((base_y + ty) * w + base_x + tx) * 4;
                        let r = *src.add(s);
                        let g = *src.add(s + 1);
                        let b = *src.add(s + 2);
                        let a = *src.add(s + 3);
                        let pixel: u16 = if a == 255 {
                            // RGB555 with high bit set.
                            0x8000 | (((r >> 3) as u16) << 10) | (((g >> 3) as u16) << 5) | ((b >> 3) as u16)
                        } else {
                            // RGBA4443 with high bit clear.
                            (((a >> 5) as u16) << 12)
                                | (((r >> 4) as u16) << 8)
                                | (((g >> 4) as u16) << 4)
                                | ((b >> 4) as u16)
                        };
                        let off = blk + (ty * BW + tx) * 2;
                        std::ptr::write(
                            dst.add(off).cast::<[u8; 2]>(),
                            [(pixel >> 8) as u8, (pixel & 0xFF) as u8],
                        );
                    }
                }
            }
        }
    }
}

fn encode_rgba8(rgba: &[u8], out: &mut [u8], w: usize, h: usize) {
    const BW: usize = 4;
    const BH: usize = 4;
    const BB: usize = 64;

    let bcx = w.div_ceil(BW);
    let bcy = h.div_ceil(BH);
    if bcx * bcy * BB > out.len() || rgba.len() < w * h * 4 {
        tracing::warn!(w, h, "encode_rgba8: buffer OOB, skipping");
        return;
    }

    let src = rgba.as_ptr();
    let dst = out.as_mut_ptr();

    // 4x4 tiles split into two 32-byte half-blocks per tile: AR pairs
    // first, then GB pairs, both indexed by `ty * BW + tx`. Inverse of
    // `decode_rgba8`'s unpack.
    for by in 0..bcy {
        let base_y = by * BH;
        let th = BH.min(h - base_y);
        for bx in 0..bcx {
            let base_x = bx * BW;
            let tw = BW.min(w - base_x);
            let blk = (by * bcx + bx) * BB;

            for ty in 0..th {
                for tx in 0..tw {
                    unsafe {
                        let s = ((base_y + ty) * w + base_x + tx) * 4;
                        let ti = ty * BW + tx;
                        let ar = blk + ti * 2;
                        let gb = blk + 32 + ti * 2;
                        std::ptr::write(dst.add(ar).cast::<[u8; 2]>(), [*src.add(s + 3), *src.add(s)]);
                        std::ptr::write(dst.add(gb).cast::<[u8; 2]>(), [*src.add(s + 1), *src.add(s + 2)]);
                    }
                }
            }
        }
    }
}
