use crate::shader_specialization::{KEY_BYTES as SHADER_KEY_BYTES, ShaderKey};
use crate::{GpuVertex, GxRenderer, helpers};
use chapa::BitField;
use gecko::flipper::gx::regs::{BlendFactor, CompareFunc, CullMode};
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) struct PipelineKey {
    pub blend_enable: bool,
    pub src_factor: BlendFactor,
    pub dst_factor: BlendFactor,
    pub subtract: bool,
    pub z_enable: bool,
    pub z_func: CompareFunc,
    pub z_write: bool,
    pub color_update: bool,
    pub alpha_update: bool,
    pub cull_mode: CullMode,
}

impl PipelineKey {
    const BYTES: usize = 10;

    fn to_bytes(self) -> [u8; Self::BYTES] {
        [
            self.blend_enable as u8,
            self.src_factor.raw(),
            self.dst_factor.raw(),
            self.subtract as u8,
            self.z_enable as u8,
            self.z_func.raw(),
            self.z_write as u8,
            self.color_update as u8,
            self.alpha_update as u8,
            self.cull_mode.raw(),
        ]
    }

    fn from_bytes(b: &[u8; Self::BYTES]) -> Self {
        Self {
            blend_enable: b[0] != 0,
            src_factor: BlendFactor::from_raw(b[1]),
            dst_factor: BlendFactor::from_raw(b[2]),
            subtract: b[3] != 0,
            z_enable: b[4] != 0,
            z_func: CompareFunc::from_raw(b[5]),
            z_write: b[6] != 0,
            color_update: b[7] != 0,
            alpha_update: b[8] != 0,
            cull_mode: CullMode::from_raw(b[9]),
        }
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) struct FullPipelineKey {
    pub shader: ShaderKey,
    pub fixed: PipelineKey,
}

pub(crate) const FULL_PIPELINE_KEY_BYTES: usize = SHADER_KEY_BYTES + PipelineKey::BYTES;
const PIPELINE_CACHE_MAGIC: [u8; 4] = *b"GPKC";
const PIPELINE_CACHE_VERSION: u32 = crate::shader_specialization::CACHE_VERSION;
pub(crate) const PIPELINE_CACHE_PATH: &str = "cache/pipeline_keys.bin";

impl FullPipelineKey {
    fn to_bytes(self) -> [u8; FULL_PIPELINE_KEY_BYTES] {
        let mut out = [0u8; FULL_PIPELINE_KEY_BYTES];
        out[..SHADER_KEY_BYTES].copy_from_slice(&self.shader.to_bytes());
        out[SHADER_KEY_BYTES..].copy_from_slice(&self.fixed.to_bytes());
        out
    }

    fn from_bytes(b: &[u8; FULL_PIPELINE_KEY_BYTES]) -> Self {
        let mut shader_bytes = [0u8; SHADER_KEY_BYTES];
        shader_bytes.copy_from_slice(&b[..SHADER_KEY_BYTES]);

        let mut fixed_bytes = [0u8; PipelineKey::BYTES];
        fixed_bytes.copy_from_slice(&b[SHADER_KEY_BYTES..]);

        Self {
            shader: ShaderKey::from_bytes(&shader_bytes),
            fixed: PipelineKey::from_bytes(&fixed_bytes),
        }
    }
}

pub(crate) fn load_cached_pipeline_keys(path: &Path) -> Vec<FullPipelineKey> {
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let mut header = [0u8; 8];
    if f.read_exact(&mut header).is_err() {
        return Vec::new();
    }

    if header[..4] != PIPELINE_CACHE_MAGIC {
        return Vec::new();
    }

    let version = u32::from_le_bytes(header[4..8].try_into().unwrap());
    if version != PIPELINE_CACHE_VERSION {
        return Vec::new();
    }

    let mut keys = Vec::new();
    let mut buf = [0u8; FULL_PIPELINE_KEY_BYTES];
    while f.read_exact(&mut buf).is_ok() {
        keys.push(FullPipelineKey::from_bytes(&buf));
    }

    keys
}

pub(crate) fn save_pipeline_keys(path: &Path, keys: &[FullPipelineKey]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let f = File::create(path)?;

    let mut w = BufWriter::new(f);
    w.write_all(&PIPELINE_CACHE_MAGIC)?;
    w.write_all(&PIPELINE_CACHE_VERSION.to_le_bytes())?;

    for k in keys {
        w.write_all(&k.to_bytes())?;
    }

    w.flush()?;

    Ok(())
}

impl GxRenderer {
    pub(crate) fn create_pipeline(
        &self,
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        key: &PipelineKey,
    ) -> wgpu::RenderPipeline {
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // position: vec3<f32>
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                // color0: vec4<f32>
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 12,
                    shader_location: 1,
                },
                // color1: vec4<f32>
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 28,
                    shader_location: 2,
                },
                // normal: vec3<f32>
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 44,
                    shader_location: 3,
                },
                // pos_view: vec3<f32>
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 56,
                    shader_location: 4,
                },
                // tex0-tex7: vec3<f32> each (s, t, q for perspective-correct interpolation)
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 68,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 80,
                    shader_location: 6,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 92,
                    shader_location: 7,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 104,
                    shader_location: 8,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 116,
                    shader_location: 9,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 128,
                    shader_location: 10,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 140,
                    shader_location: 11,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 152,
                    shader_location: 12,
                },
            ],
        };

        let blend = if key.blend_enable {
            let operation = if key.subtract {
                wgpu::BlendOperation::ReverseSubtract
            } else {
                wgpu::BlendOperation::Add
            };
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: helpers::map_blend_factor(key.src_factor),
                    dst_factor: helpers::map_blend_factor(key.dst_factor),
                    operation,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: helpers::map_blend_factor(key.src_factor),
                    dst_factor: helpers::map_blend_factor(key.dst_factor),
                    operation,
                },
            })
        } else {
            None
        };

        let depth_stencil = if key.z_enable {
            Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: key.z_write,
                depth_compare: helpers::map_compare_func(key.z_func),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            })
        } else {
            Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            })
        };

        let mut write_mask = wgpu::ColorWrites::empty();
        if key.color_update {
            write_mask |= wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN | wgpu::ColorWrites::BLUE;
        }
        if key.alpha_update {
            write_mask |= wgpu::ColorWrites::ALPHA;
        }

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gx_pipeline"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                buffers: &[vertex_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.surface_format,
                    blend,
                    write_mask,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: match key.cull_mode {
                    CullMode::Back => Some(wgpu::Face::Back),
                    CullMode::Front => Some(wgpu::Face::Front),
                    CullMode::None | CullMode::All => None,
                },
                ..Default::default()
            },
            depth_stencil,
            multisample: wgpu::MultisampleState {
                count: crate::EFB_SAMPLE_COUNT,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        })
    }
}
