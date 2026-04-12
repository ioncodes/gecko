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

/// Compute the number of raw bytes a GX texture occupies in RAM.
pub fn raw_data_size(width: u32, height: u32, format: TextureFormat) -> usize {
    let (block_w, block_h, block_bytes): (u32, u32, u32) = match format {
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
    let blocks_x = width.div_ceil(block_w);
    let blocks_y = height.div_ceil(block_h);
    (blocks_x * blocks_y * block_bytes) as usize
}

#[inline(always)]
fn put_pixel(rgba: &mut [u8], stride: usize, x: usize, y: usize, r: u8, g: u8, b: u8, a: u8) {
    let offset = (y * stride + x) * 4;
    rgba[offset] = r;
    rgba[offset + 1] = g;
    rgba[offset + 2] = b;
    rgba[offset + 3] = a;
}

#[inline(always)]
fn expand_to_8bit(value: u16, max: u16) -> u8 {
    (value * 255 / max) as u8
}

#[inline(always)]
fn rgb565_to_rgba(packed: u16) -> [u8; 4] {
    let red_5bit = (packed >> 11) & 0x1F;
    let green_6bit = (packed >> 5) & 0x3F;
    let blue_5bit = packed & 0x1F;
    [
        expand_to_8bit(red_5bit, 31),
        expand_to_8bit(green_6bit, 63),
        expand_to_8bit(blue_5bit, 31),
        255,
    ]
}

fn decode_i4(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], width: usize, height: usize) {
    const BLOCK_W: usize = 8;
    const BLOCK_H: usize = 8;
    const BLOCK_BYTES: usize = 32;

    let blocks_x = width.div_ceil(BLOCK_W);
    let blocks_y = height.div_ceil(BLOCK_H);

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let block_start = desc.ram_addr + (block_y * blocks_x + block_x) * BLOCK_BYTES;

            for tile_y in 0..BLOCK_H {
                for tile_x in 0..BLOCK_W {
                    let pixel_x = block_x * BLOCK_W + tile_x;
                    let pixel_y = block_y * BLOCK_H + tile_y;
                    if pixel_x >= width || pixel_y >= height {
                        continue;
                    }

                    let byte_offset = block_start + (tile_y * BLOCK_W + tile_x) / 2;
                    if byte_offset >= ram.len() {
                        continue;
                    }

                    let is_left_pixel = tile_x % 2 == 0;
                    let nibble = if is_left_pixel {
                        ram[byte_offset] >> 4
                    } else {
                        ram[byte_offset] & 0x0F
                    };

                    let intensity = expand_to_8bit(nibble as u16, 15);
                    put_pixel(
                        rgba, width, pixel_x, pixel_y, intensity, intensity, intensity, intensity,
                    );
                }
            }
        }
    }
}

fn decode_i8(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], width: usize, height: usize) {
    const BLOCK_W: usize = 8;
    const BLOCK_H: usize = 4;
    const BLOCK_BYTES: usize = 32;

    let blocks_x = width.div_ceil(BLOCK_W);
    let blocks_y = height.div_ceil(BLOCK_H);

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let block_start = desc.ram_addr + (block_y * blocks_x + block_x) * BLOCK_BYTES;

            for tile_y in 0..BLOCK_H {
                for tile_x in 0..BLOCK_W {
                    let pixel_x = block_x * BLOCK_W + tile_x;
                    let pixel_y = block_y * BLOCK_H + tile_y;
                    if pixel_x >= width || pixel_y >= height {
                        continue;
                    }

                    let byte_offset = block_start + tile_y * BLOCK_W + tile_x;
                    if byte_offset >= ram.len() {
                        continue;
                    }

                    let intensity = ram[byte_offset];
                    put_pixel(
                        rgba, width, pixel_x, pixel_y, intensity, intensity, intensity, intensity,
                    );
                }
            }
        }
    }
}

fn decode_ia4(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], width: usize, height: usize) {
    const BLOCK_W: usize = 8;
    const BLOCK_H: usize = 4;
    const BLOCK_BYTES: usize = 32;

    let blocks_x = width.div_ceil(BLOCK_W);
    let blocks_y = height.div_ceil(BLOCK_H);

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let block_start = desc.ram_addr + (block_y * blocks_x + block_x) * BLOCK_BYTES;

            for tile_y in 0..BLOCK_H {
                for tile_x in 0..BLOCK_W {
                    let pixel_x = block_x * BLOCK_W + tile_x;
                    let pixel_y = block_y * BLOCK_H + tile_y;
                    if pixel_x >= width || pixel_y >= height {
                        continue;
                    }

                    let byte_offset = block_start + tile_y * BLOCK_W + tile_x;
                    if byte_offset >= ram.len() {
                        continue;
                    }

                    let packed = ram[byte_offset];
                    let alpha = expand_to_8bit((packed >> 4) as u16, 15);
                    let intensity = expand_to_8bit((packed & 0x0F) as u16, 15);
                    put_pixel(rgba, width, pixel_x, pixel_y, intensity, intensity, intensity, alpha);
                }
            }
        }
    }
}

fn decode_ia8(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], width: usize, height: usize) {
    const BLOCK_W: usize = 4;
    const BLOCK_H: usize = 4;
    const BLOCK_BYTES: usize = 32;

    let blocks_x = width.div_ceil(BLOCK_W);
    let blocks_y = height.div_ceil(BLOCK_H);

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let block_start = desc.ram_addr + (block_y * blocks_x + block_x) * BLOCK_BYTES;

            for tile_y in 0..BLOCK_H {
                for tile_x in 0..BLOCK_W {
                    let pixel_x = block_x * BLOCK_W + tile_x;
                    let pixel_y = block_y * BLOCK_H + tile_y;
                    if pixel_x >= width || pixel_y >= height {
                        continue;
                    }

                    let byte_offset = block_start + (tile_y * BLOCK_W + tile_x) * 2;
                    if byte_offset + 1 >= ram.len() {
                        continue;
                    }

                    let alpha = ram[byte_offset];
                    let intensity = ram[byte_offset + 1];
                    put_pixel(rgba, width, pixel_x, pixel_y, intensity, intensity, intensity, alpha);
                }
            }
        }
    }
}

fn decode_rgb565(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], width: usize, height: usize) {
    const BLOCK_W: usize = 4;
    const BLOCK_H: usize = 4;
    const BLOCK_BYTES: usize = 32;

    let blocks_x = width.div_ceil(BLOCK_W);
    let blocks_y = height.div_ceil(BLOCK_H);

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let block_start = desc.ram_addr + (block_y * blocks_x + block_x) * BLOCK_BYTES;

            for tile_y in 0..BLOCK_H {
                for tile_x in 0..BLOCK_W {
                    let pixel_x = block_x * BLOCK_W + tile_x;
                    let pixel_y = block_y * BLOCK_H + tile_y;
                    if pixel_x >= width || pixel_y >= height {
                        continue;
                    }

                    let byte_offset = block_start + (tile_y * BLOCK_W + tile_x) * 2;
                    if byte_offset + 1 >= ram.len() {
                        continue;
                    }

                    let [r, g, b, a] = rgb565_to_rgba(u16::from_be_bytes([ram[byte_offset], ram[byte_offset + 1]]));
                    put_pixel(rgba, width, pixel_x, pixel_y, r, g, b, a);
                }
            }
        }
    }
}

fn decode_rgb5a3(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], width: usize, height: usize) {
    const BLOCK_W: usize = 4;
    const BLOCK_H: usize = 4;
    const BLOCK_BYTES: usize = 32;

    let blocks_x = width.div_ceil(BLOCK_W);
    let blocks_y = height.div_ceil(BLOCK_H);

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let block_start = desc.ram_addr + (block_y * blocks_x + block_x) * BLOCK_BYTES;

            for tile_y in 0..BLOCK_H {
                for tile_x in 0..BLOCK_W {
                    let pixel_x = block_x * BLOCK_W + tile_x;
                    let pixel_y = block_y * BLOCK_H + tile_y;
                    if pixel_x >= width || pixel_y >= height {
                        continue;
                    }

                    let byte_offset = block_start + (tile_y * BLOCK_W + tile_x) * 2;
                    if byte_offset + 1 >= ram.len() {
                        continue;
                    }

                    let packed = u16::from_be_bytes([ram[byte_offset], ram[byte_offset + 1]]);
                    let has_no_alpha = packed & 0x8000 != 0;

                    if has_no_alpha {
                        let r = expand_to_8bit((packed >> 10) & 0x1F, 31);
                        let g = expand_to_8bit((packed >> 5) & 0x1F, 31);
                        let b = expand_to_8bit(packed & 0x1F, 31);
                        put_pixel(rgba, width, pixel_x, pixel_y, r, g, b, 255);
                    } else {
                        let a = expand_to_8bit((packed >> 12) & 0x7, 7);
                        let r = expand_to_8bit((packed >> 8) & 0xF, 15);
                        let g = expand_to_8bit((packed >> 4) & 0xF, 15);
                        let b = expand_to_8bit(packed & 0xF, 15);
                        put_pixel(rgba, width, pixel_x, pixel_y, r, g, b, a);
                    }
                }
            }
        }
    }
}

fn decode_rgba8(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], width: usize, height: usize) {
    const BLOCK_W: usize = 4;
    const BLOCK_H: usize = 4;
    const BLOCK_BYTES: usize = 64;

    let blocks_x = width.div_ceil(BLOCK_W);
    let blocks_y = height.div_ceil(BLOCK_H);

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let block_start = desc.ram_addr + (block_y * blocks_x + block_x) * BLOCK_BYTES;

            for tile_y in 0..BLOCK_H {
                for tile_x in 0..BLOCK_W {
                    let pixel_x = block_x * BLOCK_W + tile_x;
                    let pixel_y = block_y * BLOCK_H + tile_y;
                    if pixel_x >= width || pixel_y >= height {
                        continue;
                    }

                    let texel_index = tile_y * BLOCK_W + tile_x;
                    let ar_offset = block_start + texel_index * 2;
                    let gb_offset = block_start + 32 + texel_index * 2;
                    if gb_offset + 1 >= ram.len() {
                        continue;
                    }

                    let alpha = ram[ar_offset];
                    let red = ram[ar_offset + 1];
                    let green = ram[gb_offset];
                    let blue = ram[gb_offset + 1];
                    put_pixel(rgba, width, pixel_x, pixel_y, red, green, blue, alpha);
                }
            }
        }
    }
}

fn decode_cmpr(ram: &[u8], desc: &TextureDescriptor, rgba: &mut [u8], width: usize, height: usize) {
    const MACRO_W: usize = 8;
    const MACRO_H: usize = 8;
    const MACRO_BYTES: usize = 32;
    const SUB_BLOCK_BYTES: usize = 8;

    let blocks_x = width.div_ceil(MACRO_W);
    let blocks_y = height.div_ceil(MACRO_H);

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let macro_start = desc.ram_addr + (block_y * blocks_x + block_x) * MACRO_BYTES;

            for sub_index in 0..4usize {
                let sub_start = macro_start + sub_index * SUB_BLOCK_BYTES;
                let sub_x_offset = (sub_index % 2) * 4;
                let sub_y_offset = (sub_index / 2) * 4;

                if sub_start + 7 >= ram.len() {
                    continue;
                }

                let color0 = u16::from_be_bytes([ram[sub_start], ram[sub_start + 1]]);
                let color1 = u16::from_be_bytes([ram[sub_start + 2], ram[sub_start + 3]]);
                let palette = build_dxt1_palette(color0, color1);

                for row in 0..4usize {
                    let index_byte = ram[sub_start + 4 + row];
                    for col in 0..4usize {
                        let pixel_x = block_x * MACRO_W + sub_x_offset + col;
                        let pixel_y = block_y * MACRO_H + sub_y_offset + row;
                        if pixel_x >= width || pixel_y >= height {
                            continue;
                        }

                        let shift = (3 - col) * 2;
                        let palette_index = ((index_byte >> shift) & 0x03) as usize;
                        let [r, g, b, a] = palette[palette_index];
                        put_pixel(rgba, width, pixel_x, pixel_y, r, g, b, a);
                    }
                }
            }
        }
    }
}

fn build_dxt1_palette(color0: u16, color1: u16) -> [[u8; 4]; 4] {
    let [r0, g0, b0, _] = rgb565_to_rgba(color0);
    let [r1, g1, b1, _] = rgb565_to_rgba(color1);

    let blend = |a: u8, b: u8, weight_a: u16, weight_b: u16| -> u8 {
        ((a as u16 * weight_a + b as u16 * weight_b) / (weight_a + weight_b)) as u8
    };

    let mut palette = [[0u8; 4]; 4];
    palette[0] = [r0, g0, b0, 255];
    palette[1] = [r1, g1, b1, 255];

    if color0 > color1 {
        palette[2] = [blend(r0, r1, 2, 1), blend(g0, g1, 2, 1), blend(b0, b1, 2, 1), 255];
        palette[3] = [blend(r0, r1, 1, 2), blend(g0, g1, 1, 2), blend(b0, b1, 1, 2), 255];
    } else {
        palette[2] = [blend(r0, r1, 1, 1), blend(g0, g1, 1, 1), blend(b0, b1, 1, 1), 255];
        palette[3] = [0, 0, 0, 0];
    }

    palette
}
