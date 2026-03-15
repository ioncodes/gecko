use gekko::flipper::gx::regs::{BlendFactor, CompareFunc, MagFilter, MinFilter, TevRegisterH, TevRegisterL, WrapMode};

pub fn map_wrap_mode(wrap: WrapMode) -> wgpu::AddressMode {
    match wrap {
        WrapMode::Clamp => wgpu::AddressMode::ClampToEdge,
        WrapMode::Repeat => wgpu::AddressMode::Repeat,
        WrapMode::Mirror => wgpu::AddressMode::MirrorRepeat,
    }
}

pub fn map_mag_filter(filter: MagFilter) -> wgpu::FilterMode {
    match filter {
        MagFilter::Nearest => wgpu::FilterMode::Nearest,
        MagFilter::Linear => wgpu::FilterMode::Linear,
    }
}

pub fn map_min_filter(filter: MinFilter) -> wgpu::FilterMode {
    match filter {
        MinFilter::Nearest | MinFilter::NearestMipmapNearest | MinFilter::NearestMipmapLinear => {
            wgpu::FilterMode::Nearest
        }
        MinFilter::Linear | MinFilter::LinearMipmapNearest | MinFilter::LinearMipmapLinear => wgpu::FilterMode::Linear,
    }
}

pub fn map_blend_factor(f: BlendFactor) -> wgpu::BlendFactor {
    match f {
        BlendFactor::Zero => wgpu::BlendFactor::Zero,
        BlendFactor::One => wgpu::BlendFactor::One,
        BlendFactor::SrcClr => wgpu::BlendFactor::Src,
        BlendFactor::InvSrcClr => wgpu::BlendFactor::OneMinusSrc,
        BlendFactor::SrcAlpha => wgpu::BlendFactor::SrcAlpha,
        BlendFactor::InvSrcAlpha => wgpu::BlendFactor::OneMinusSrcAlpha,
        BlendFactor::DstAlpha => wgpu::BlendFactor::DstAlpha,
        BlendFactor::InvDstAlpha => wgpu::BlendFactor::OneMinusDstAlpha,
    }
}

pub fn map_compare_func(f: CompareFunc) -> wgpu::CompareFunction {
    match f {
        CompareFunc::Never => wgpu::CompareFunction::Never,
        CompareFunc::Less => wgpu::CompareFunction::Less,
        CompareFunc::Equal => wgpu::CompareFunction::Equal,
        CompareFunc::LessEqual => wgpu::CompareFunction::LessEqual,
        CompareFunc::Greater => wgpu::CompareFunction::Greater,
        CompareFunc::NotEqual => wgpu::CompareFunction::NotEqual,
        CompareFunc::GreaterEqual => wgpu::CompareFunction::GreaterEqual,
        CompareFunc::Always => wgpu::CompareFunction::Always,
    }
}

pub fn s11_to_f32(val: u16) -> f32 {
    let signed = if val & 0x400 != 0 {
        val as i32 - 0x800
    } else {
        val as i32
    };
    signed as f32 / 255.0
}

pub fn decode_tev_color_regs(lo: &[TevRegisterL; 4], hi: &[TevRegisterH; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for i in 0..4 {
        out[i] = [
            s11_to_f32(lo[i].r()),
            s11_to_f32(hi[i].g()),
            s11_to_f32(hi[i].b()),
            s11_to_f32(lo[i].a()),
        ];
    }
    out
}

pub fn unpack_tev_orders(orders: &[u32; 8]) -> [u32; 16] {
    let mut out = [0u32; 16];
    for stage in 0..16u32 {
        let reg_idx = (stage / 2) as usize;
        if reg_idx < 8 {
            let raw = orders[reg_idx];
            if stage % 2 == 0 {
                out[stage as usize] = raw & 0x3FF; // low 10 bits
            } else {
                out[stage as usize] = ((raw >> 12) & 0x7)
                    | (((raw >> 15) & 0x7) << 3)
                    | (((raw >> 18) & 0x1) << 6)
                    | (((raw >> 19) & 0x7) << 7);
            }
        }
    }
    out
}
