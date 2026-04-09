use crate::BindGroupCacheKey;
use crate::pipeline::PipelineKey;
use crate::triangulate::{self, GpuVertex, align_up};
use crate::{DrawUniforms, FrameUniforms, GxRenderer, helpers, texture};
use encase::{ShaderType as _, UniformBuffer};
use gecko::flipper::gx::draw::DrawCommands;
use gecko::flipper::gx::regs::{MagFilter, MinFilter, WrapMode};
use glam::{Mat4, UVec4, Vec4};

impl GxRenderer {
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

        self.prepare_resources(device, queue, commands, ram);

        let (frame_uniform_bytes, draw_call_indices) = self.aggregate_draw_data(device, commands);

        if self.scratch_draws.is_empty() {
            return;
        }

        self.upload_buffers(device, queue, &frame_uniform_bytes);
        self.execute_render_pass(device, queue, commands, target, &draw_call_indices);
    }

    fn prepare_resources(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, commands: &DrawCommands, ram: &[u8]) {
        for dc in &commands.commands {
            for desc in dc.textures.iter().flatten() {
                let key = (desc.ram_addr, desc.width, desc.height, desc.format);
                self.texture_cache.entry(key).or_insert_with(|| {
                    let (tex, view) = texture::upload_texture(device, queue, ram, desc);
                    (tex, view)
                });
                let sampler_key = (desc.wrap_s, desc.wrap_t, desc.mag_filter, desc.min_filter);
                self.sampler_cache.entry(sampler_key).or_insert_with(|| {
                    device.create_sampler(&wgpu::SamplerDescriptor {
                        label: Some("gx_sampler"),
                        address_mode_u: helpers::map_wrap_mode(sampler_key.0),
                        address_mode_v: helpers::map_wrap_mode(sampler_key.1),
                        mag_filter: helpers::map_mag_filter(sampler_key.2),
                        min_filter: helpers::map_min_filter(sampler_key.3),
                        ..Default::default()
                    })
                });
            }
            let pipeline_key = PipelineKey::from_draw_call(dc);
            if !self.pipeline_cache.contains_key(&pipeline_key) {
                let pipeline = self.create_pipeline(device, &pipeline_key);
                self.pipeline_cache.insert(pipeline_key, pipeline);
            }
        }

        let fallback_sampler_key = (WrapMode::Clamp, WrapMode::Clamp, MagFilter::Linear, MinFilter::Linear);
        self.sampler_cache.entry(fallback_sampler_key).or_insert_with(|| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("gx_sampler_fallback"),
                ..Default::default()
            })
        });
    }

    fn aggregate_draw_data(&mut self, device: &wgpu::Device, commands: &DrawCommands) -> (Vec<u8>, Vec<usize>) {
        self.scratch_vertices.clear();
        self.scratch_draws.clear();
        self.scratch_uniform_bytes.clear();

        let draw_stride = self.draw_uniform_stride as usize;
        let draw_encase_size = DrawUniforms::min_size().get() as usize;
        let frame_stride = align_up(
            FrameUniforms::min_size().get(),
            device.limits().min_uniform_buffer_offset_alignment as u64,
        ) as usize;
        let frame_encase_size = FrameUniforms::min_size().get() as usize;
        let mut frame_uniform_bytes: Vec<u8> = Vec::new();
        let mut draw_call_indices: Vec<usize> = Vec::new();

        for (dc_idx, dc) in commands.commands.iter().enumerate() {
            let prev_len = self.scratch_vertices.len();
            triangulate::triangulate_into(dc, &mut self.scratch_vertices);
            let added = self.scratch_vertices.len() - prev_len;
            if added == 0 {
                continue;
            }

            let mvp = commands.projection * dc.modelview;
            let draw_uniform = DrawUniforms {
                mvp: Mat4::from_cols_array_2d(&mvp.0),
            };

            let start = self.scratch_draws.len() * draw_stride;
            self.scratch_uniform_bytes.resize(start + draw_stride, 0);
            let mut draw_buf = UniformBuffer::new(&mut self.scratch_uniform_bytes[start..start + draw_encase_size]);
            draw_buf.write(&draw_uniform).unwrap();

            let alpha_cmp = dc.bp_alpha_compare;
            let frame_uniform = FrameUniforms {
                tev_color_regs: dc.tev_color_regs.map(Vec4::from),
                tev_konst_colors: dc.tev_konst_colors.map(Vec4::from),
                tev_color_env: pack_u32x16_to_uvec4x4(&dc.tev_color_env.map(|e| e.raw())),
                tev_alpha_env: pack_u32x16_to_uvec4x4(&dc.tev_alpha_env.map(|e| e.raw())),
                tev_orders: pack_u32x16_to_uvec4x4(&dc.tev_orders.map(|o| o.raw())),
                num_tev_stages: dc.num_tev_stages as u32,
                alpha_ref0: alpha_cmp.ref0() as f32 / 255.0,
                alpha_ref1: alpha_cmp.ref1() as f32 / 255.0,
                alpha_comp0: alpha_cmp.comp0() as u32,
                alpha_comp1: alpha_cmp.comp1() as u32,
                alpha_op: alpha_cmp.op() as u32,
                light_colors: dc.light_colors.map(Vec4::from),
                light_cosatt: dc.light_cosatt.map(Vec4::from),
                light_distatt: dc.light_distatt.map(Vec4::from),
                light_pos: dc.light_pos.map(Vec4::from),
                light_dir: dc.light_dir.map(Vec4::from),
                color_ctrl: dc.color_ctrl.raw(),
                alpha_ctrl: dc.alpha_ctrl.raw(),
                ambient_color: Vec4::from(dc.ambient_color),
                material_color: Vec4::from(dc.material_color),
            };

            let fstart = self.scratch_draws.len() * frame_stride;
            frame_uniform_bytes.resize(fstart + frame_stride, 0);
            let mut frame_buf = UniformBuffer::new(&mut frame_uniform_bytes[fstart..fstart + frame_encase_size]);
            frame_buf.write(&frame_uniform).unwrap();

            self.scratch_draws.push((prev_len as u32, added as u32));
            draw_call_indices.push(dc_idx);
        }

        (frame_uniform_bytes, draw_call_indices)
    }

    fn upload_buffers(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame_uniform_bytes: &[u8]) {
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

        let frame_stride = align_up(
            FrameUniforms::min_size().get(),
            device.limits().min_uniform_buffer_offset_alignment as u64,
        ) as usize;
        let needed_frame_size = (num_draws * frame_stride) as u64;
        if needed_frame_size > self.frame_uniform_buffer.size() {
            self.frame_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gx_frame_uniforms"),
                size: needed_frame_size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.bind_group_cache.clear();
        }

        queue.write_buffer(&self.frame_uniform_buffer, 0, frame_uniform_bytes);
        queue.write_buffer(&self.draw_uniform_buffer, 0, &self.scratch_uniform_bytes);
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.scratch_vertices));
    }

    fn execute_render_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        commands: &DrawCommands,
        target: &wgpu::TextureView,
        draw_call_indices: &[usize],
    ) {
        let fallback_sampler_key = (WrapMode::Clamp, WrapMode::Clamp, MagFilter::Linear, MinFilter::Linear);
        let frame_stride = align_up(
            FrameUniforms::min_size().get(),
            device.limits().min_uniform_buffer_offset_alignment as u64,
        ) as usize;

        // Pre-build bind groups for each unique texture/sampler configuration.
        for &dc_idx in draw_call_indices {
            let dc = &commands.commands[dc_idx];
            let mut tex_keys: [Option<_>; 8] = [None; 8];
            let mut sampler_keys: [Option<_>; 8] = [None; 8];

            for slot in 0..8 {
                if let Some(desc) = &dc.textures[slot] {
                    tex_keys[slot] = Some((desc.ram_addr, desc.width, desc.height, desc.format));
                    sampler_keys[slot] = Some((desc.wrap_s, desc.wrap_t, desc.mag_filter, desc.min_filter));
                }
            }

            let bg_key = BindGroupCacheKey { tex_keys, sampler_keys };
            self.bind_group_cache.entry(bg_key).or_insert_with(|| {
                let mut tex_views: [&wgpu::TextureView; 8] = [&self.fallback_view; 8];
                let mut tex_samplers: [&wgpu::Sampler; 8] = [&self.sampler_cache[&fallback_sampler_key]; 8];

                for slot in 0..8 {
                    if let Some(tk) = &tex_keys[slot] {
                        tex_views[slot] = &self.texture_cache[tk].1;
                        tex_samplers[slot] = &self.sampler_cache[&sampler_keys[slot].unwrap()];
                    }
                }

                let entries: [wgpu::BindGroupEntry; 18] = std::array::from_fn(|i| match i {
                    0 => wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.frame_uniform_buffer,
                            offset: 0,
                            size: Some(FrameUniforms::min_size()),
                        }),
                    },
                    1 => wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.draw_uniform_buffer,
                            offset: 0,
                            size: Some(DrawUniforms::min_size()),
                        }),
                    },
                    2..=9 => wgpu::BindGroupEntry {
                        binding: i as u32,
                        resource: wgpu::BindingResource::TextureView(tex_views[i - 2]),
                    },
                    _ => wgpu::BindGroupEntry {
                        binding: i as u32,
                        resource: wgpu::BindingResource::Sampler(tex_samplers[i - 10]),
                    },
                });

                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.bind_group_layout,
                    entries: &entries,
                })
            });
        }

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

                let mut tex_keys: [Option<_>; 8] = [None; 8];
                let mut sampler_keys: [Option<_>; 8] = [None; 8];
                for slot in 0..8 {
                    if let Some(desc) = &dc.textures[slot] {
                        tex_keys[slot] = Some((desc.ram_addr, desc.width, desc.height, desc.format));
                        sampler_keys[slot] = Some((desc.wrap_s, desc.wrap_t, desc.mag_filter, desc.min_filter));
                    }
                }

                let bg_key = BindGroupCacheKey { tex_keys, sampler_keys };
                let bind_group = &self.bind_group_cache[&bg_key];

                let frame_offset = (index * frame_stride) as u32;
                let draw_offset = (index as u64 * self.draw_uniform_stride) as u32;
                rpass.set_bind_group(0, bind_group, &[frame_offset, draw_offset]);
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
        self.bind_group_cache.clear();
    }
}

fn pack_u32x16_to_uvec4x4(data: &[u32; 16]) -> [UVec4; 4] {
    [
        UVec4::new(data[0], data[1], data[2], data[3]),
        UVec4::new(data[4], data[5], data[6], data[7]),
        UVec4::new(data[8], data[9], data[10], data[11]),
        UVec4::new(data[12], data[13], data[14], data[15]),
    ]
}
