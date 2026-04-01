use crate::{
    DrawUniforms, FrameUniforms, GxRenderer, helpers,
    pipeline::PipelineKey,
    texture,
    triangulate::{self, GpuVertex, align_up},
};
use gecko::flipper::gx::draw::DrawCommands;
use gecko::flipper::gx::regs::{MagFilter, MinFilter, WrapMode};

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
    }

    fn aggregate_draw_data(&mut self, device: &wgpu::Device, commands: &DrawCommands) -> (Vec<u8>, Vec<usize>) {
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
            triangulate::triangulate_into(dc, &mut self.scratch_vertices);
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
            std::mem::size_of::<FrameUniforms>() as u64,
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
        }

        queue.write_buffer(&self.frame_uniform_buffer, 0, frame_uniform_bytes);
        queue.write_buffer(&self.draw_uniform_buffer, 0, &self.scratch_uniform_bytes);
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.scratch_vertices));
    }

    fn execute_render_pass(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        commands: &DrawCommands,
        target: &wgpu::TextureView,
        draw_call_indices: &[usize],
    ) {
        let fallback_sampler_key = (WrapMode::Clamp, WrapMode::Clamp, MagFilter::Linear, MinFilter::Linear);
        let frame_stride = align_up(
            std::mem::size_of::<FrameUniforms>() as u64,
            device.limits().min_uniform_buffer_offset_alignment as u64,
        ) as usize;

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
                let mut tex_samplers: [&wgpu::Sampler; 8] = [&self.sampler_cache[&fallback_sampler_key]; 8];
                for slot in 0..8 {
                    if let Some(desc) = &dc.textures[slot] {
                        let tex_key = (desc.ram_addr, desc.width, desc.height, desc.format);
                        let sampler_key = (desc.wrap_s, desc.wrap_t, desc.mag_filter, desc.min_filter);
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
