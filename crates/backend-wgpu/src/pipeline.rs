use crate::{GxRenderer, helpers, triangulate::GpuVertex};
use gecko::flipper::gx::draw::DrawCall;
use gecko::flipper::gx::regs::{BlendFactor, CompareFunc};

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) struct PipelineKey {
    pub blend_enable: bool,
    pub src_factor: BlendFactor,
    pub dst_factor: BlendFactor,
    pub subtract: bool,
    pub z_enable: bool,
    pub z_func: CompareFunc,
    pub z_write: bool,
}

impl PipelineKey {
    pub fn from_draw_call(dc: &DrawCall) -> Self {
        let blend = dc.bp_blend_mode;
        let zmode = dc.bp_zmode;
        PipelineKey {
            blend_enable: blend.blend_enable(),
            src_factor: blend.src_factor(),
            dst_factor: blend.dst_factor(),
            subtract: blend.subtract(),
            z_enable: zmode.enable(),
            z_func: zmode.func(),
            z_write: zmode.update_enable(),
        }
    }
}

impl GxRenderer {
    pub(crate) fn create_pipeline(&self, device: &wgpu::Device, key: &PipelineKey) -> wgpu::RenderPipeline {
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
                // color: vec4<f32>
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 12,
                    shader_location: 1,
                },
                // tex0-tex7: vec2<f32> each
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 28,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 36,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 44,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 52,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 60,
                    shader_location: 6,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 68,
                    shader_location: 7,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 76,
                    shader_location: 8,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 84,
                    shader_location: 9,
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

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gx_pipeline"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs_main"),
                buffers: &[vertex_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.surface_format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }
}
