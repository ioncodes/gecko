use crate::shader_specialization::ShaderKey;
use crate::{GpuVertex, GxRenderer, helpers};
use gecko::flipper::gx::regs::{BlendFactor, CompareFunc, CullMode};

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

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) struct FullPipelineKey {
    pub shader: ShaderKey,
    pub fixed: PipelineKey,
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
