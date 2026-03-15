mod helpers;
mod texture;

use gekko::flipper::gx::draw::{DrawCall, DrawCommands, Primitive, TextureFormat};
use gekko::flipper::gx::regs::{BlendFactor, CompareFunc, MagFilter, MinFilter, WrapMode};
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
    mvp: [[f32; 4]; 4],            // 64B
    tev_color_regs: [[f32; 4]; 4], // 64B — TEVPREV, TEVREG0-2 as float RGBA
    tev_color_env: [u32; 16],      // 64B — raw bitfields per stage
    tev_alpha_env: [u32; 16],      // 64B — raw bitfields per stage
    tev_stage_orders: [u32; 16],   // 64B — pre-unpacked per-stage order
    num_tev_stages: u32,           // 4B
    alpha_ref0: f32,               // 4B
    alpha_ref1: f32,               // 4B
    alpha_comp0: u32,              // 4B
    alpha_comp1: u32,              // 4B
    alpha_op: u32,                 // 4B
    _padding: [u32; 2],            // 8B — pad to 16-byte alignment
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

pub struct GxRenderer {
    pipeline_cache: HashMap<PipelineKey, wgpu::RenderPipeline>,
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    surface_format: wgpu::TextureFormat,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    uniform_stride: u64,
    uniform_capacity: usize,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    depth_width: u32,
    depth_height: u32,
    /// Cache: (wrap_s, wrap_t, mag_filter, min_filter) -> wgpu Sampler
    sampler_cache: HashMap<(WrapMode, WrapMode, MagFilter, MinFilter), wgpu::Sampler>,
    /// Cache: (ram_addr, width, height, format) -> (wgpu Texture, TextureView)
    texture_cache: HashMap<(usize, u32, u32, TextureFormat), (wgpu::Texture, wgpu::TextureView)>,
    /// Fallback 1x1 white texture view used when no texture is bound
    fallback_view: wgpu::TextureView,
    // Scratch buffers reused across frames to avoid per-frame allocations
    scratch_vertices: Vec<GpuVertex>,
    scratch_draws: Vec<(u32, u32)>,
    scratch_uniform_bytes: Vec<u8>,
}

impl GxRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let uniform_size = std::mem::size_of::<Uniforms>() as u64;
        let uniform_stride = align_up(uniform_size, device.limits().min_uniform_buffer_offset_alignment as u64);

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
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(uniform_size),
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
            size: uniform_stride,
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
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniform_buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(uniform_size),
                    }),
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
            uniform_stride,
            uniform_capacity: 1,
            vertex_buffer,
            vertex_capacity: initial_capacity,
            depth_texture,
            depth_view,
            depth_width,
            depth_height,
            sampler_cache: HashMap::from([(
                (WrapMode::Clamp, WrapMode::Clamp, MagFilter::Linear, MinFilter::Linear),
                sampler,
            )]),
            texture_cache: HashMap::new(),
            fallback_view,
            scratch_vertices: Vec::new(),
            scratch_draws: Vec::new(),
            scratch_uniform_bytes: Vec::new(),
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
        let tev_color_regs = helpers::decode_tev_color_regs(&commands.tev_color_regs_lo, &commands.tev_color_regs_hi);
        let tev_color_env = commands.tev_color_env.map(|e| e.raw());
        let tev_alpha_env = commands.tev_alpha_env.map(|e| e.raw());
        let tev_stage_orders = helpers::unpack_tev_orders(&commands.tev_orders);
        let num_tev_stages = commands.num_tev_stages as u32;

        // Get or create the pipeline for current blend/z state
        let pipeline_key = PipelineKey::from_draw_commands(commands);
        if !self.pipeline_cache.contains_key(&pipeline_key) {
            let pipeline = self.create_pipeline(device, &pipeline_key);
            self.pipeline_cache.insert(pipeline_key, pipeline);
        }

        // Upload texture slot 0 if present, using cache to avoid redundant uploads
        let sampler_key = if let Some(desc) = &commands.textures[0] {
            let key = (desc.ram_addr, desc.width, desc.height, desc.format);
            if !self.texture_cache.contains_key(&key) {
                let (tex, view) = texture::upload_texture(device, queue, ram, desc);
                self.texture_cache.insert(key, (tex, view));
            }
            (desc.wrap_s, desc.wrap_t, desc.mag_filter, desc.min_filter)
        } else {
            (WrapMode::Clamp, WrapMode::Clamp, MagFilter::Linear, MinFilter::Linear)
        };

        // Get or create sampler for these modes
        if !self.sampler_cache.contains_key(&sampler_key) {
            let s = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("gx_sampler"),
                address_mode_u: helpers::map_wrap_mode(sampler_key.0),
                address_mode_v: helpers::map_wrap_mode(sampler_key.1),
                mag_filter: helpers::map_mag_filter(sampler_key.2),
                min_filter: helpers::map_min_filter(sampler_key.3),
                ..Default::default()
            });
            self.sampler_cache.insert(sampler_key, s);
        }
        self.scratch_vertices.clear();
        self.scratch_draws.clear();
        self.scratch_uniform_bytes.clear();

        let stride = self.uniform_stride as usize;
        let uniform_size = std::mem::size_of::<Uniforms>();

        for dc in &commands.commands {
            let prev_len = self.scratch_vertices.len();
            triangulate_into(dc, &mut self.scratch_vertices);
            let added = self.scratch_vertices.len() - prev_len;
            if added == 0 {
                continue;
            }

            let mvp = commands.projection * dc.modelview;
            let uniform = Uniforms {
                mvp: mvp.0,
                tev_color_regs,
                tev_color_env,
                tev_alpha_env,
                tev_stage_orders,
                num_tev_stages,
                alpha_ref0: alpha_cmp.ref0() as f32 / 255.0,
                alpha_ref1: alpha_cmp.ref1() as f32 / 255.0,
                alpha_comp0: alpha_cmp.comp0() as u32,
                alpha_comp1: alpha_cmp.comp1() as u32,
                alpha_op: alpha_cmp.op() as u32,
                _padding: [0; 2],
            };
            let start = self.scratch_draws.len() * stride;
            self.scratch_uniform_bytes.resize(start + stride, 0);
            self.scratch_uniform_bytes[start..start + uniform_size].copy_from_slice(bytemuck::bytes_of(&uniform));
            self.scratch_draws.push((prev_len as u32, added as u32));
        }

        if self.scratch_draws.is_empty() {
            return;
        }

        self.ensure_uniform_capacity(device, self.scratch_draws.len());

        if self.scratch_vertices.len() > self.vertex_capacity {
            self.vertex_capacity = self.scratch_vertices.len().next_power_of_two();
            self.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gx_vertices"),
                size: (self.vertex_capacity * std::mem::size_of::<GpuVertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        let sampler = &self.sampler_cache[&sampler_key];
        let tex_view = if commands.textures[0].is_some() {
            let desc = commands.textures[0].as_ref().unwrap();
            let key = (desc.ram_addr, desc.width, desc.height, desc.format);
            &self.texture_cache[&key].1
        } else {
            &self.fallback_view
        };

        queue.write_buffer(&self.uniform_buffer, 0, &self.scratch_uniform_bytes);
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.scratch_vertices));

        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniform_buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(std::mem::size_of::<Uniforms>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(tex_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        let pipeline = &self.pipeline_cache[&pipeline_key];

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gx_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            rpass.set_pipeline(pipeline);
            rpass.set_vertex_buffer(0, self.vertex_buffer.slice(..));

            for (index, (first_vertex, vertex_count)) in self.scratch_draws.iter().copied().enumerate() {
                let uniform_offset = (index as u64 * self.uniform_stride) as u32;
                rpass.set_bind_group(0, &self.bind_group, &[uniform_offset]);
                rpass.draw(first_vertex..first_vertex + vertex_count, 0..1);
            }
        }

        queue.submit([encoder.finish()]);
    }

    fn ensure_uniform_capacity(&mut self, device: &wgpu::Device, count: usize) {
        if count <= self.uniform_capacity {
            return;
        }

        self.uniform_capacity = count.next_power_of_two();
        self.uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx_uniforms"),
            size: self.uniform_stride * self.uniform_capacity as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
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

fn triangulate_into(dc: &DrawCall, out: &mut Vec<GpuVertex>) {
    match dc.primitive {
        Primitive::Triangles => {
            out.extend(dc.vertices.iter().map(GpuVertex::from));
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
}

fn align_up(value: u64, alignment: u64) -> u64 {
    (value + alignment - 1) & !(alignment - 1)
}
