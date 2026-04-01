mod helpers;
pub mod texture;

use gecko::flipper::gx::draw::{DrawCall, DrawCommands, Primitive, TextureFormat};
use gecko::flipper::gx::regs::{BlendFactor, CompareFunc, MagFilter, MinFilter, WrapMode};
use std::collections::HashMap;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuVertex {
    position: [f32; 3],
    color: [f32; 4],
    tex0: [f32; 2],
    tex1: [f32; 2],
    tex2: [f32; 2],
    tex3: [f32; 2],
    tex4: [f32; 2],
    tex5: [f32; 2],
    tex6: [f32; 2],
    tex7: [f32; 2],
}

impl From<&gecko::flipper::gx::draw::Vertex> for GpuVertex {
    fn from(v: &gecko::flipper::gx::draw::Vertex) -> Self {
        let tc = |i: usize| v.texcoords[i].unwrap_or([0.0, 0.0]);
        Self {
            position: v.position,
            color: v.color0,
            tex0: tc(0),
            tex1: tc(1),
            tex2: tc(2),
            tex3: tc(3),
            tex4: tc(4),
            tex5: tc(5),
            tex6: tc(6),
            tex7: tc(7),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameUniforms {
    tev_color_regs: [[f32; 4]; 4],
    tev_konst_colors: [[f32; 4]; 16],
    tev_color_env: [u32; 16],
    tev_alpha_env: [u32; 16],
    tev_orders: [u32; 16],
    num_tev_stages: u32,
    alpha_ref0: f32,
    alpha_ref1: f32,
    alpha_comp0: u32,
    alpha_comp1: u32,
    alpha_op: u32,
    _padding: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DrawUniforms {
    mvp: [[f32; 4]; 4],
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
    fn from_draw_call(dc: &DrawCall) -> Self {
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

pub struct GxRenderer {
    pipeline_cache: HashMap<PipelineKey, wgpu::RenderPipeline>,
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    surface_format: wgpu::TextureFormat,
    bind_group_layout: wgpu::BindGroupLayout,
    #[allow(dead_code)]
    bind_group: wgpu::BindGroup,
    frame_uniform_buffer: wgpu::Buffer,
    draw_uniform_buffer: wgpu::Buffer,
    draw_uniform_stride: u64,
    draw_uniform_capacity: usize,
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
        let frame_uniform_size = std::mem::size_of::<FrameUniforms>() as u64;
        let draw_uniform_size = std::mem::size_of::<DrawUniforms>() as u64;
        let draw_uniform_stride = align_up(
            draw_uniform_size,
            device.limits().min_uniform_buffer_offset_alignment as u64,
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gx_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        // Bindings: 0=FrameUniforms, 1=DrawUniforms, 2-9=textures 0-7, 10-17=samplers 0-7
        let mut layout_entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(frame_uniform_size),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(draw_uniform_size),
                },
                count: None,
            },
        ];
        for i in 0..8u32 {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 2 + i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        for i in 0..8u32 {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 10 + i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &layout_entries,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });

        let frame_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx_frame_uniforms"),
            size: frame_uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let draw_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx_draw_uniforms"),
            size: draw_uniform_stride,
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

        let mut initial_entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &frame_uniform_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(frame_uniform_size),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &draw_uniform_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(draw_uniform_size),
                }),
            },
        ];
        for i in 0..8u32 {
            initial_entries.push(wgpu::BindGroupEntry {
                binding: 2 + i,
                resource: wgpu::BindingResource::TextureView(&fallback_view),
            });
        }
        for i in 0..8u32 {
            initial_entries.push(wgpu::BindGroupEntry {
                binding: 10 + i,
                resource: wgpu::BindingResource::Sampler(&sampler),
            });
        }
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bind_group_layout,
            entries: &initial_entries,
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
            frame_uniform_buffer,
            draw_uniform_buffer,
            draw_uniform_stride,
            draw_uniform_capacity: 1,
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
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 28, shader_location: 2 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 36, shader_location: 3 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 44, shader_location: 4 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 52, shader_location: 5 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 60, shader_location: 6 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 68, shader_location: 7 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 76, shader_location: 8 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 84, shader_location: 9 },
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

        // Ensure all referenced textures are uploaded and pipelines/samplers created
        for dc in &commands.commands {
            for desc in dc.textures.iter().flatten() {
                let key = (desc.ram_addr, desc.width, desc.height, desc.format);
                if !self.texture_cache.contains_key(&key) {
                    let (tex, view) = texture::upload_texture(device, queue, ram, desc);
                    self.texture_cache.insert(key, (tex, view));
                }
                let sampler_key = (desc.wrap_s, desc.wrap_t, desc.mag_filter, desc.min_filter);
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
            }
            let pipeline_key = PipelineKey::from_draw_call(dc);
            if !self.pipeline_cache.contains_key(&pipeline_key) {
                let pipeline = self.create_pipeline(device, &pipeline_key);
                self.pipeline_cache.insert(pipeline_key, pipeline);
            }
        }

        // Ensure fallback sampler exists
        let fallback_sampler_key = (WrapMode::Clamp, WrapMode::Clamp, MagFilter::Linear, MinFilter::Linear);
        if !self.sampler_cache.contains_key(&fallback_sampler_key) {
            let s = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("gx_sampler_fallback"),
                ..Default::default()
            });
            self.sampler_cache.insert(fallback_sampler_key, s);
        }

        self.scratch_vertices.clear();
        self.scratch_draws.clear();
        self.scratch_uniform_bytes.clear();

        let draw_stride = self.draw_uniform_stride as usize;
        let draw_size = std::mem::size_of::<DrawUniforms>();
        let frame_stride = align_up(
            std::mem::size_of::<FrameUniforms>() as u64,
            device.limits().min_uniform_buffer_offset_alignment as u64,
        ) as usize;
        let frame_size = std::mem::size_of::<FrameUniforms>();
        let mut frame_uniform_bytes: Vec<u8> = Vec::new();
        let mut draw_call_indices: Vec<usize> = Vec::new();

        for (dc_idx, dc) in commands.commands.iter().enumerate() {
            let prev_len = self.scratch_vertices.len();
            triangulate_into(dc, &mut self.scratch_vertices);
            let added = self.scratch_vertices.len() - prev_len;
            if added == 0 {
                continue;
            }

            let mvp = commands.projection * dc.modelview;
            let draw_uniform = DrawUniforms { mvp: mvp.0 };
            let start = self.scratch_draws.len() * draw_stride;
            self.scratch_uniform_bytes.resize(start + draw_stride, 0);
            self.scratch_uniform_bytes[start..start + draw_size].copy_from_slice(bytemuck::bytes_of(&draw_uniform));

            let alpha_cmp = dc.bp_alpha_compare;
            let frame_uniform = FrameUniforms {
                tev_color_regs: dc.tev_color_regs,
                tev_konst_colors: dc.tev_konst_colors,
                tev_color_env: dc.tev_color_env.map(|e| e.raw()),
                tev_alpha_env: dc.tev_alpha_env.map(|e| e.raw()),
                tev_orders: dc.tev_orders.map(|o| o.raw()),
                num_tev_stages: dc.num_tev_stages as u32,
                alpha_ref0: alpha_cmp.ref0() as f32 / 255.0,
                alpha_ref1: alpha_cmp.ref1() as f32 / 255.0,
                alpha_comp0: alpha_cmp.comp0() as u32,
                alpha_comp1: alpha_cmp.comp1() as u32,
                alpha_op: alpha_cmp.op() as u32,
                _padding: [0; 2],
            };
            let fstart = self.scratch_draws.len() * frame_stride;
            frame_uniform_bytes.resize(fstart + frame_stride, 0);
            frame_uniform_bytes[fstart..fstart + frame_size].copy_from_slice(bytemuck::bytes_of(&frame_uniform));

            self.scratch_draws.push((prev_len as u32, added as u32));
            draw_call_indices.push(dc_idx);
        }

        if self.scratch_draws.is_empty() {
            return;
        }

        let num_draws = self.scratch_draws.len();
        self.ensure_draw_capacity(device, num_draws);

        if self.scratch_vertices.len() > self.vertex_capacity {
            self.vertex_capacity = self.scratch_vertices.len().next_power_of_two();
            self.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gx_vertices"),
                size: (self.vertex_capacity * std::mem::size_of::<GpuVertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        let needed_frame_size = (num_draws * frame_stride) as u64;
        if needed_frame_size > self.frame_uniform_buffer.size() {
            self.frame_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gx_frame_uniforms"),
                size: needed_frame_size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        queue.write_buffer(&self.frame_uniform_buffer, 0, &frame_uniform_bytes);
        queue.write_buffer(&self.draw_uniform_buffer, 0, &self.scratch_uniform_bytes);
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.scratch_vertices));

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
            rpass.set_vertex_buffer(0, self.vertex_buffer.slice(..));

            for (index, (first_vertex, vertex_count)) in self.scratch_draws.iter().copied().enumerate() {
                let dc = &commands.commands[draw_call_indices[index]];
                let pipeline_key = PipelineKey::from_draw_call(dc);
                let pipeline = &self.pipeline_cache[&pipeline_key];
                rpass.set_pipeline(pipeline);

                // Resolve all 8 texture views + samplers for this draw call
                let mut tex_views: [&wgpu::TextureView; 8] = [&self.fallback_view; 8];
                let mut tex_samplers: [&wgpu::Sampler; 8] =
                    [&self.sampler_cache[&fallback_sampler_key]; 8];
                for slot in 0..8 {
                    if let Some(desc) = &dc.textures[slot] {
                        let tex_key = (desc.ram_addr, desc.width, desc.height, desc.format);
                        let sampler_key =
                            (desc.wrap_s, desc.wrap_t, desc.mag_filter, desc.min_filter);
                        tex_views[slot] = &self.texture_cache[&tex_key].1;
                        tex_samplers[slot] = &self.sampler_cache[&sampler_key];
                    }
                }

                let frame_offset = (index as u64 * frame_stride as u64) as wgpu::BufferAddress;
                let draw_offset = (index as u64 * self.draw_uniform_stride) as u32;

                let mut entries = vec![
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.frame_uniform_buffer,
                            offset: frame_offset,
                            size: wgpu::BufferSize::new(std::mem::size_of::<FrameUniforms>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.draw_uniform_buffer,
                            offset: 0,
                            size: wgpu::BufferSize::new(std::mem::size_of::<DrawUniforms>() as u64),
                        }),
                    },
                ];
                for i in 0..8u32 {
                    entries.push(wgpu::BindGroupEntry {
                        binding: 2 + i,
                        resource: wgpu::BindingResource::TextureView(tex_views[i as usize]),
                    });
                }
                for i in 0..8u32 {
                    entries.push(wgpu::BindGroupEntry {
                        binding: 10 + i,
                        resource: wgpu::BindingResource::Sampler(tex_samplers[i as usize]),
                    });
                }
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.bind_group_layout,
                    entries: &entries,
                });

                rpass.set_bind_group(0, &bind_group, &[draw_offset]);
                rpass.draw(first_vertex..first_vertex + vertex_count, 0..1);
            }
        }

        queue.submit([encoder.finish()]);
    }

    fn ensure_draw_capacity(&mut self, device: &wgpu::Device, count: usize) {
        if count <= self.draw_uniform_capacity {
            return;
        }

        self.draw_uniform_capacity = count.next_power_of_two();
        self.draw_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gx_draw_uniforms"),
            size: self.draw_uniform_stride * self.draw_uniform_capacity as u64,
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
        Primitive::TriangleStrip => {
            for i in 2..dc.vertices.len() {
                if i % 2 == 0 {
                    out.push((&dc.vertices[i - 2]).into());
                    out.push((&dc.vertices[i - 1]).into());
                    out.push((&dc.vertices[i]).into());
                } else {
                    out.push((&dc.vertices[i - 1]).into());
                    out.push((&dc.vertices[i - 2]).into());
                    out.push((&dc.vertices[i]).into());
                }
            }
        }
        Primitive::TriangleFan => {
            for i in 2..dc.vertices.len() {
                out.push((&dc.vertices[0]).into());
                out.push((&dc.vertices[i - 1]).into());
                out.push((&dc.vertices[i]).into());
            }
        }
        _ => unimplemented!("triangulation for {:?}", dc.primitive),
    }
}

fn align_up(value: u64, alignment: u64) -> u64 {
    (value + alignment - 1) & !(alignment - 1)
}
