use gecko::flipper::gx::draw::{TextureDescriptor, TextureFormat, TlutFormat};
use gecko::flipper::gx::regs::{MagFilter, MinFilter, WrapMode};
use gecko::flipper::gx::texture;

pub fn decode(encoded: image::banner::Texture<'_>) -> Option<Vec<u8>> {
    let format = match encoded.format {
        0 => TextureFormat::I4,
        1 => TextureFormat::I8,
        2 => TextureFormat::IA4,
        3 => TextureFormat::IA8,
        4 => TextureFormat::RGB565,
        5 => TextureFormat::RGB5A3,
        6 => TextureFormat::RGBA8,
        8 => TextureFormat::CI4,
        9 => TextureFormat::CI8,
        10 => TextureFormat::CI14,
        14 => TextureFormat::CMPR,
        _ => return None,
    };

    let size = texture::raw_data_size(encoded.width, encoded.height, format);
    let pixels = encoded.pixels.get(..size)?;

    let (palette, tlut) = match encoded.palette {
        Some(palette) => {
            let tlut = match palette.format {
                0 => TlutFormat::IA8,
                1 => TlutFormat::RGB565,
                2 => TlutFormat::RGB5A3,
                _ => return None,
            };
            let entries = palette
                .entries
                .chunks_exact(2)
                .map(|p| u16::from_be_bytes([p[0], p[1]]))
                .collect::<Vec<_>>();
            (entries, tlut)
        }
        None => (Vec::new(), TlutFormat::IA8),
    };

    let desc = TextureDescriptor {
        ram_addr: 0,
        width: encoded.width,
        height: encoded.height,
        format,
        wrap_s: WrapMode::Clamp,
        wrap_t: WrapMode::Clamp,
        mag_filter: MagFilter::Nearest,
        min_filter: MinFilter::Nearest,
    };
    Some(texture::decode_to_rgba(pixels, &desc, &palette, tlut))
}
