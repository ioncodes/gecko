@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var palette: texture_2d<f32>;

fn fetch(pos: vec2<f32>) -> vec4<f32> {
    return textureLoad(source, vec2<i32>(pos), 0);
}

fn lookup(index: u32) -> vec4<f32> {
    return textureLoad(palette, vec2<i32>(i32(index), 0), 0);
}

@fragment
fn fs_palette_r8(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return lookup(u32(round(fetch(p.xy).r * 255.0)));
}

@fragment
fn fs_palette_i8(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return lookup(luma_u8(fetch(p.xy)));
}
