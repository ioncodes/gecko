struct DecodeInput {
    width: u32,
    height: u32,
    format: u32,
    tlut_format: u32,
    tile_w: u32,
    tile_h: u32,
    tile_bytes: u32,
    data_offset: u32,
    palette_offset: u32,
    palette_len: u32,
    words: array<u32>,
}

@group(0) @binding(0) var<storage, read> source: DecodeInput;
@group(0) @binding(1) var dest: texture_storage_2d<rgba8unorm, write>;

fn byte_at(offset: u32) -> u32 {
    let address = source.data_offset + offset;
    return (source.words[address / 4u] >> ((address % 4u) * 8u)) & 255u;
}

fn be16(offset: u32) -> u32 {
    return (byte_at(offset) << 8u) | byte_at(offset + 1u);
}

fn expand3(v: u32) -> u32 { return (v << 5u) | (v << 2u) | (v >> 1u); }
fn expand4(v: u32) -> u32 { return (v << 4u) | v; }
fn expand5(v: u32) -> u32 { return (v << 3u) | (v >> 2u); }
fn expand6(v: u32) -> u32 { return (v << 2u) | (v >> 4u); }

fn rgb565(v: u32) -> vec4<u32> {
    return vec4<u32>(expand5((v >> 11u) & 31u), expand6((v >> 5u) & 63u), expand5(v & 31u), 255u);
}

fn rgb5a3(v: u32) -> vec4<u32> {
    if (v & 32768u) != 0u {
        return vec4<u32>(expand5((v >> 10u) & 31u), expand5((v >> 5u) & 31u), expand5(v & 31u), 255u);
    }
    return vec4<u32>(expand4((v >> 8u) & 15u), expand4((v >> 4u) & 15u), expand4(v & 15u), expand3((v >> 12u) & 7u));
}

fn ia8(v: u32) -> vec4<u32> {
    return vec4<u32>(vec3<u32>(v & 255u), v >> 8u);
}

fn palette(index: u32) -> vec4<u32> {
    var entry = 0u;
    if index < source.palette_len {
        entry = source.words[source.palette_offset + index];
    }

    switch source.tlut_format {
        case 1u: { return rgb565(entry); }
        case 2u: { return rgb5a3(entry); }
        default: { return ia8(entry); }
    }
}

fn cmpr(block: u32, local: vec2<u32>) -> vec4<u32> {
    let sub = block + ((local.y / 4u) * 2u + local.x / 4u) * 8u;
    let c0 = be16(sub);
    let c1 = be16(sub + 2u);
    let p0 = rgb565(c0);
    let p1 = rgb565(c1);
    let index = (byte_at(sub + 4u + local.y % 4u) >> ((3u - local.x % 4u) * 2u)) & 3u;

    if index == 0u { return p0; }
    if index == 1u { return p1; }

    if c0 > c1 {
        if index == 2u { return vec4<u32>((2u * p0.rgb + p1.rgb) / 3u, 255u); }
        return vec4<u32>((p0.rgb + 2u * p1.rgb) / 3u, 255u);
    }

    return vec4<u32>((p0.rgb + p1.rgb) / 2u, select(255u, 0u, index == 3u));
}

fn texel(pos: vec2<u32>) -> vec4<u32> {
    let tile = vec2<u32>(source.tile_w, source.tile_h);
    let blocks_x = (source.width + tile.x - 1u) / tile.x;
    let block = ((pos.y / tile.y) * blocks_x + pos.x / tile.x) * source.tile_bytes;
    let local = pos % tile;
    let index = local.y * tile.x + local.x;

    switch source.format {
        case 0u, 8u: {
            let nibble = (byte_at(block + index / 2u) >> select(0u, 4u, index % 2u == 0u)) & 15u;
            if source.format == 8u { return palette(nibble); }
            return vec4<u32>(expand4(nibble));
        }
        case 1u: { return vec4<u32>(byte_at(block + index)); }
        case 2u: {
            let v = byte_at(block + index);
            return vec4<u32>(vec3<u32>(expand4(v & 15u)), expand4(v >> 4u));
        }
        case 3u: { return ia8(be16(block + index * 2u)); }
        case 4u: { return rgb565(be16(block + index * 2u)); }
        case 5u: { return rgb5a3(be16(block + index * 2u)); }
        case 6u: {
            let ar = block + index * 2u;
            let gb = ar + 32u;
            return vec4<u32>(byte_at(ar + 1u), byte_at(gb), byte_at(gb + 1u), byte_at(ar));
        }
        case 9u: { return palette(byte_at(block + index)); }
        case 10u: { return palette(be16(block + index * 2u) & 16383u); }
        case 14u: { return cmpr(block, local); }
        default: { return vec4<u32>(0u); }
    }
}

@compute @workgroup_size(8, 8)
fn decode(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= source.width || id.y >= source.height { return; }
    textureStore(dest, vec2<i32>(id.xy), vec4<f32>(texel(id.xy)) / 255.0);
}
