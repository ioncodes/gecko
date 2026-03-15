mod texture;

use gekko::flipper::gx::draw::{DrawCall, DrawCommands, Primitive, TextureFormat};
use gekko::flipper::gx::regs::{BlendFactor, CompareFunc};
use std::collections::HashMap;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuVertex {
    position: [f32; 3],
    color: [f32; 4],
    tex0: [f32; 2],
}

impl From<&gekko::flipper::gx::draw::Vertex> for GpuVertex {
    fn from(v: &gekko::flipper::gx::draw::Vertex) -> Self {
        Self {
            position: v.position,
            color: v.color0,
            tex0: v.tex0.unwrap_or([0.0, 0.0]),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
    tev_color_source: u32, // 0 = vertex color, 1 = texture
    alpha_ref0: f32,
    alpha_ref1: f32,
    alpha_comp0: u32, // CompareFunc as u32
    alpha_comp1: u32,
    alpha_op: u32, // AlphaOp as u32
    _padding: [u32; 2],
}

const SHADER: &str = include_str!("gx.wgsl");

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct PipelineKey {
    blend_enable: bool,
    src_factor: BlendFactor,
    dst_factor: BlendFactor,
    subtract: bool,
    z_enable: bool,
    z_func: CompareFunc,
    z_write: bool,
}

impl PipelineKey {
    fn from_draw_commands(commands: &DrawCommands) -> Self {
        let blend = commands.bp_blend_mode;
        let zmode = commands.bp_zmode;
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

fn map_blend_factor(f: BlendFactor) -> wgpu::BlendFactor {
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

fn map_compare_func(f: CompareFunc) -> wgpu::CompareFunction {
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

pub struct GxRenderer {
    pipeline_cache: HashMap<PipelineKey, wgpu::RenderPipeline>,
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    surface_format: wgpu::TextureFormat,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    depth_width: u32,
    depth_height: u32,
    sampler: wgpu::Sampler,
    /// Cache: (ram_addr, width, height, format) -> (wgpu Texture, TextureView)
    texture_cache: HashMap<(usize, u32, u32, TextureFormat), (wgpu::Texture, wgpu::TextureView)>,
    /// Fallback 1x1 white texture view used when no texture is bound
    fallback_view: wgpu::TextureView,
}

impl GxRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gx_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx_uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gx_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let fallback_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gx_fallback_tex"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // White pixel
        queue.write_texture(
            fallback_texture.as_image_copy(),
            &[255u8, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let fallback_view = fallback_texture.create_view(&Default::default());

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&fallback_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let initial_capacity = 1024;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx_vertices"),
            size: (initial_capacity * std::mem::size_of::<GpuVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let depth_width = width.max(1);
        let depth_height = height.max(1);
        let (depth_texture, depth_view) = create_depth_texture(device, depth_width, depth_height);

        GxRenderer {
            pipeline_cache: HashMap::new(),
            shader,
            pipeline_layout,
            surface_format,
            bind_group_layout,
            bind_group,
            uniform_buffer,
            vertex_buffer,
            vertex_capacity: initial_capacity,
            depth_texture,
            depth_view,
            depth_width,
            depth_height,
            sampler,
            texture_cache: HashMap::new(),
            fallback_view,
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.ensure_depth_texture(device, width, height);
    }

    fn ensure_depth_texture(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if (width, height) == (self.depth_width, self.depth_height) {
            return;
        }
        let (tex, view) = create_depth_texture(device, width, height);
        self.depth_texture = tex;
        self.depth_view = view;
        self.depth_width = width;
        self.depth_height = height;
    }

    fn create_pipeline(&self, device: &wgpu::Device, key: &PipelineKey) -> wgpu::RenderPipeline {
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 12,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 28,
                    shader_location: 2,
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
                    src_factor: map_blend_factor(key.src_factor),
                    dst_factor: map_blend_factor(key.dst_factor),
                    operation,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: map_blend_factor(key.src_factor),
                    dst_factor: map_blend_factor(key.dst_factor),
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
                depth_compare: map_compare_func(key.z_func),
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

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        commands: &DrawCommands,
        ram: &[u8],
        target: &wgpu::TextureView,
        target_width: u32,
        target_height: u32,
    ) {
        self.ensure_depth_texture(device, target_width, target_height);

        if commands.commands.is_empty() {
            return;
        }

        let alpha_cmp = commands.bp_alpha_compare;
        let tev_color_source = commands.textures[0].is_some() as u32;

        // Get or create the pipeline for current blend/z state
        let pipeline_key = PipelineKey::from_draw_commands(commands);
        if !self.pipeline_cache.contains_key(&pipeline_key) {
            let pipeline = self.create_pipeline(device, &pipeline_key);
            self.pipeline_cache.insert(pipeline_key, pipeline);
        }

        // Upload texture slot 0 if present, using cache to avoid redundant uploads
        let tex_view = if let Some(desc) = &commands.textures[0] {
            let key = (desc.ram_addr, desc.width, desc.height, desc.format);
            if !self.texture_cache.contains_key(&key) {
                let (tex, view) = texture::upload_texture(device, queue, ram, desc);
                self.texture_cache.insert(key, (tex, view));
            }
            &self.texture_cache[&key].1
        } else {
            &self.fallback_view
        };

        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(tex_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let pipeline = &self.pipeline_cache[&pipeline_key];

        for (i, dc) in commands.commands.iter().enumerate() {
            let vertices = triangulate(dc);
            if vertices.is_empty() {
                continue;
            }

            let mvp = commands.projection * dc.modelview;

            queue.write_buffer(
                &self.uniform_buffer,
                0,
                bytemuck::bytes_of(&Uniforms {
                    mvp: mvp.0,
                    tev_color_source,
                    alpha_ref0: alpha_cmp.ref0() as f32 / 255.0,
                    alpha_ref1: alpha_cmp.ref1() as f32 / 255.0,
                    alpha_comp0: alpha_cmp.comp0() as u32,
                    alpha_comp1: alpha_cmp.comp1() as u32,
                    alpha_op: alpha_cmp.op() as u32,
                    _padding: [0; 2],
                }),
            );

            if vertices.len() > self.vertex_capacity {
                self.vertex_capacity = vertices.len().next_power_of_two();
                self.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gx_vertices"),
                    size: (self.vertex_capacity * std::mem::size_of::<GpuVertex>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }

            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));

            let color_load = if i == 0 {
                wgpu::LoadOp::Clear(wgpu::Color::BLACK)
            } else {
                wgpu::LoadOp::Load
            };
            let depth_load = if i == 0 {
                wgpu::LoadOp::Clear(1.0)
            } else {
                wgpu::LoadOp::Load
            };

            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("gx_render_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: color_load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: depth_load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    occlusion_query_set: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                });
                rpass.set_pipeline(pipeline);
                rpass.set_bind_group(0, &self.bind_group, &[]);
                rpass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                rpass.draw(0..vertices.len() as u32, 0..1);
            }
            queue.submit([encoder.finish()]);
        }
    }
}

fn create_depth_texture(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gx_depth"),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth24Plus,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    (tex, view)
}

fn triangulate(dc: &DrawCall) -> Vec<GpuVertex> {
    let mut out = Vec::new();
    match dc.primitive {
        Primitive::Triangles => {
            for v in &dc.vertices {
                out.push(v.into());
            }
        }
        Primitive::Quads => {
            for quad in dc.vertices.chunks(4) {
                if quad.len() < 4 {
                    continue;
                }
                out.push((&quad[0]).into());
                out.push((&quad[1]).into());
                out.push((&quad[2]).into());
                out.push((&quad[0]).into());
                out.push((&quad[2]).into());
                out.push((&quad[3]).into());
            }
        }
        _ => unimplemented!("triangulation for {:?}", dc.primitive),
    }
    out
}
