use crate::pipeline::PipelineKey;
use crate::{BindGroupCacheKey, DrawUniforms, FrameUniforms, GxRenderer, SamplerKey, helpers};
use crate::{GpuVertex, align_up};
use encase::{ShaderType as _, UniformBuffer};
use gecko::flipper::gx::regs::{MagFilter, MinFilter, WrapMode};
use gecko::host::{DrawData, GxAction};
use glam::{Mat4, UVec4, Vec4};

fn pack_u32_slice_to_uvec4x4(data: &[u32]) -> [UVec4; 4] {
    let get = |i: usize| if i < data.len() { data[i] } else { 0 };
    [
        UVec4::new(get(0), get(1), get(2), get(3)),
        UVec4::new(get(4), get(5), get(6), get(7)),
        UVec4::new(get(8), get(9), get(10), get(11)),
        UVec4::new(get(12), get(13), get(14), get(15)),
    ]
}

impl GxRenderer {
    fn current_pipeline_key(&self) -> PipelineKey {
        let blend = self.current_blend_mode;
        let zmode = self.current_zmode;
        PipelineKey {
            blend_enable: blend.blend_enable(),
            src_factor: blend.src_factor(),
            dst_factor: blend.dst_factor(),
            subtract: blend.subtract(),
            z_enable: zmode.enable(),
            z_func: zmode.func(),
            z_write: zmode.update_enable(),
            color_update: blend.color_update(),
            alpha_update: blend.alpha_update(),
            cull_mode: self.current_cull_mode,
        }
    }

    /// Build a bind-group cache key from the current tracked textures.
    fn current_bind_group_key(&self) -> BindGroupCacheKey {
        BindGroupCacheKey {
            tex_keys: self.current_texture_ids,
            sampler_keys: self.current_sampler_keys,
        }
    }

    /// Process a batch of [`GxAction`] values, rendering draw calls into the
    /// internal EFB and handling EFB copies / XFB presentation.
    pub fn render_actions(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, actions: &[GxAction]) {
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

        // Per-draw snapshots of pipeline key + bind group key (captured at draw time
        // from the current tracked state).
        let mut draw_pipeline_keys: Vec<PipelineKey> = Vec::new();
        let mut draw_bg_keys: Vec<BindGroupCacheKey> = Vec::new();
        // Per-draw viewport/scissor snapshots (captured from tracked state at draw time).
        let mut draw_viewports: Vec<gecko::flipper::gx::draw::Viewport> = Vec::new();
        let mut draw_scissors: Vec<gecko::flipper::gx::draw::Scissor> = Vec::new();

        for action in actions {
            match action {
                GxAction::SetProjection { matrix, .. } => {
                    self.current_projection = Mat4::from_cols_array_2d(matrix);
                }
                GxAction::SetViewport(vp) => {
                    self.current_viewport = *vp;
                }
                GxAction::SetScissor(sc) => {
                    self.current_scissor = *sc;
                }
                GxAction::SetDepthMode(zm) => {
                    self.current_zmode = *zm;
                }
                GxAction::SetBlendMode(bm) => {
                    self.current_blend_mode = *bm;
                }
                GxAction::SetAlphaCompare(ac) => {
                    self.current_alpha_compare = *ac;
                }
                GxAction::LoadTexture {
                    id,
                    width,
                    height,
                    rgba,
                } => {
                    let tex = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("gx_tex"),
                        size: wgpu::Extent3d {
                            width: *width,
                            height: *height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    queue.write_texture(
                        tex.as_image_copy(),
                        rgba,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(*width * 4),
                            rows_per_image: None,
                        },
                        wgpu::Extent3d {
                            width: *width,
                            height: *height,
                            depth_or_array_layers: 1,
                        },
                    );
                    let view = tex.create_view(&Default::default());
                    // Invalidate bind groups that referenced the old texture.
                    let tid = *id;
                    self.bind_group_cache
                        .retain(|key, _| !key.tex_keys.iter().any(|k| *k == Some(tid)));
                    self.texture_cache.insert(*id, (tex, view));
                }
                GxAction::SetTexture {
                    slot,
                    id,
                    wrap_s,
                    wrap_t,
                    mag_filter,
                    min_filter,
                } => {
                    self.current_texture_ids[*slot] = Some(*id);
                    let sampler_key: SamplerKey = (*wrap_s, *wrap_t, *mag_filter, *min_filter);
                    self.current_sampler_keys[*slot] = Some(sampler_key);
                    self.ensure_sampler(device, &sampler_key);
                }
                GxAction::SetCullMode(mode) => {
                    self.current_cull_mode = *mode;
                }

                GxAction::Draw(draw) => {
                    // Ensure pipeline for current tracked blend/depth state.
                    let pipeline_key = self.current_pipeline_key();
                    if !self.pipeline_cache.contains_key(&pipeline_key) {
                        let pipeline = self.create_pipeline(device, &pipeline_key);
                        self.pipeline_cache.insert(pipeline_key, pipeline);
                    }

                    // Triangulate.
                    let prev_len = self.scratch_vertices.len();
                    triangulate_draw_data(draw, &mut self.scratch_vertices);
                    let added = self.scratch_vertices.len() - prev_len;
                    if added == 0 {
                        continue;
                    }

                    // Build per-draw uniform (MVP) using tracked projection.
                    let modelview = Mat4::from_cols_array_2d(&draw.modelview);
                    let mvp = self.current_projection * modelview;
                    let draw_uniform = DrawUniforms { mvp };

                    let start = self.scratch_draws.len() * draw_stride;
                    self.scratch_uniform_bytes.resize(start + draw_stride, 0);
                    let mut draw_buf =
                        UniformBuffer::new(&mut self.scratch_uniform_bytes[start..start + draw_encase_size]);
                    draw_buf.write(&draw_uniform).unwrap();

                    // Build per-draw frame uniform (TEV + lighting from DrawData,
                    // alpha compare from tracked state).
                    let alpha_cmp = self.current_alpha_compare;
                    let frame_uniform = FrameUniforms {
                        tev_color_regs: draw.tev_color_regs.map(Vec4::from),
                        tev_konst_colors: draw.tev_konst_colors.map(Vec4::from),
                        tev_color_env: pack_u32_slice_to_uvec4x4(&draw.tev_color_env),
                        tev_alpha_env: pack_u32_slice_to_uvec4x4(&draw.tev_alpha_env),
                        tev_orders: pack_u32_slice_to_uvec4x4(&draw.tev_orders),
                        num_tev_stages: draw.num_tev_stages as u32,
                        alpha_ref0: alpha_cmp.ref0() as f32 / 255.0,
                        alpha_ref1: alpha_cmp.ref1() as f32 / 255.0,
                        alpha_comp0: alpha_cmp.comp0() as u32,
                        alpha_comp1: alpha_cmp.comp1() as u32,
                        alpha_op: alpha_cmp.op() as u32,
                        light_colors: draw.lights.each_ref().map(|l| Vec4::from(l.color)),
                        light_cosatt: draw.lights.each_ref().map(|l| Vec4::from(l.cosatt)),
                        light_distatt: draw.lights.each_ref().map(|l| Vec4::from(l.distatt)),
                        light_pos: draw.lights.each_ref().map(|l| Vec4::from(l.position)),
                        light_dir: draw.lights.each_ref().map(|l| Vec4::from(l.direction)),
                        color_ctrl0: draw.color_ctrl[0].raw(),
                        alpha_ctrl0: draw.alpha_ctrl[0].raw(),
                        color_ctrl1: draw.color_ctrl[1].raw(),
                        alpha_ctrl1: draw.alpha_ctrl[1].raw(),
                        ambient_color0: Vec4::from(draw.ambient_color[0]),
                        ambient_color1: Vec4::from(draw.ambient_color[1]),
                        material_color0: Vec4::from(draw.material_color[0]),
                        material_color1: Vec4::from(draw.material_color[1]),
                    };

                    let fstart = self.scratch_draws.len() * frame_stride;
                    frame_uniform_bytes.resize(fstart + frame_stride, 0);
                    let mut frame_buf =
                        UniformBuffer::new(&mut frame_uniform_bytes[fstart..fstart + frame_encase_size]);
                    frame_buf.write(&frame_uniform).unwrap();

                    // Snapshot tracked state for this draw.
                    draw_pipeline_keys.push(pipeline_key);
                    draw_bg_keys.push(self.current_bind_group_key());
                    draw_viewports.push(self.current_viewport);
                    draw_scissors.push(self.current_scissor);

                    self.scratch_draws.push((prev_len as u32, added as u32));
                }

                GxAction::CopyEfb(copy) => {
                    // Flush pending draws before the copy.
                    if !self.scratch_draws.is_empty() {
                        self.upload_buffers(device, queue, &frame_uniform_bytes);
                        self.execute_action_render_pass(
                            device,
                            queue,
                            &draw_pipeline_keys,
                            &draw_bg_keys,
                            &draw_viewports,
                            &draw_scissors,
                        );
                        self.scratch_vertices.clear();
                        self.scratch_draws.clear();
                        self.scratch_uniform_bytes.clear();
                        frame_uniform_bytes.clear();
                        draw_pipeline_keys.clear();
                        draw_bg_keys.clear();
                        draw_viewports.clear();
                        draw_scissors.clear();
                    }

                    self.execute_efb_copy(device, queue, copy);
                }

                GxAction::CopyXfb {
                    id,
                    src_x,
                    src_y,
                    src_w,
                    src_h,
                    clear,
                    clear_color,
                    clear_z,
                } => {
                    if !self.scratch_draws.is_empty() {
                        self.upload_buffers(device, queue, &frame_uniform_bytes);
                        self.execute_action_render_pass(
                            device,
                            queue,
                            &draw_pipeline_keys,
                            &draw_bg_keys,
                            &draw_viewports,
                            &draw_scissors,
                        );
                        self.scratch_vertices.clear();
                        self.scratch_draws.clear();
                        self.scratch_uniform_bytes.clear();
                        frame_uniform_bytes.clear();
                        draw_pipeline_keys.clear();
                        draw_bg_keys.clear();
                        draw_viewports.clear();
                        draw_scissors.clear();
                    }

                    self.execute_copy_xfb(
                        device,
                        queue,
                        *id,
                        *src_x,
                        *src_y,
                        *src_w,
                        *src_h,
                        *clear,
                        *clear_color,
                        *clear_z,
                    );
                }

                GxAction::PresentXfb { width, height, parts } => {
                    // Flush any trailing draws.
                    if !self.scratch_draws.is_empty() {
                        self.upload_buffers(device, queue, &frame_uniform_bytes);
                        self.execute_action_render_pass(
                            device,
                            queue,
                            &draw_pipeline_keys,
                            &draw_bg_keys,
                            &draw_viewports,
                            &draw_scissors,
                        );
                        self.scratch_vertices.clear();
                        self.scratch_draws.clear();
                        self.scratch_uniform_bytes.clear();
                        frame_uniform_bytes.clear();
                        draw_pipeline_keys.clear();
                        draw_bg_keys.clear();
                        draw_viewports.clear();
                        draw_scissors.clear();
                    }

                    self.execute_present_xfb(device, queue, *width, *height, parts);
                }
            }
        }

        // Flush remaining draws.
        if !self.scratch_draws.is_empty() {
            self.upload_buffers(device, queue, &frame_uniform_bytes);
            self.execute_action_render_pass(
                device,
                queue,
                &draw_pipeline_keys,
                &draw_bg_keys,
                &draw_viewports,
                &draw_scissors,
            );
        }
    }

    /// Render pass for streamed action draws.
    fn execute_action_render_pass(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline_keys: &[PipelineKey],
        bg_keys: &[BindGroupCacheKey],
        viewports: &[gecko::flipper::gx::draw::Viewport],
        scissors: &[gecko::flipper::gx::draw::Scissor],
    ) {
        let target_width = crate::EFB_WIDTH;
        let target_height = crate::EFB_HEIGHT;
        let fallback_sampler_key = (WrapMode::Clamp, WrapMode::Clamp, MagFilter::Linear, MinFilter::Linear);
        let frame_stride = align_up(
            FrameUniforms::min_size().get(),
            device.limits().min_uniform_buffer_offset_alignment as u64,
        ) as usize;

        // Pre-build bind groups from tracked texture snapshots.
        for bg_key in bg_keys {
            self.bind_group_cache.entry(bg_key.clone()).or_insert_with(|| {
                let mut tex_views: [&wgpu::TextureView; 8] = [&self.fallback_view; 8];
                let mut tex_samplers: [&wgpu::Sampler; 8] = [&self.sampler_cache[&fallback_sampler_key]; 8];

                for slot in 0..8 {
                    if let Some(tid) = &bg_key.tex_keys[slot] {
                        if let Some((_, view)) = self.texture_cache.get(tid) {
                            tex_views[slot] = view;
                        }
                        if let Some(sk) = &bg_key.sampler_keys[slot] {
                            if let Some(sampler) = self.sampler_cache.get(sk) {
                                tex_samplers[slot] = sampler;
                            }
                        }
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
            let needs_initial_clear = self.efb_needs_clear;
            self.efb_needs_clear = false;
            let color_load = if needs_initial_clear {
                wgpu::LoadOp::Clear(wgpu::Color::BLACK)
            } else {
                wgpu::LoadOp::Load
            };
            let depth_load = if needs_initial_clear {
                wgpu::LoadOp::Clear(1.0)
            } else {
                wgpu::LoadOp::Load
            };

            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gx_action_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.efb_msaa_view,
                    resolve_target: Some(&self.efb_view),
                    ops: wgpu::Operations {
                        load: color_load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.efb_depth_view,
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
            rpass.set_vertex_buffer(0, self.vertex_buffer.slice(..));

            for (index, (first_vertex, vertex_count)) in self.scratch_draws.iter().copied().enumerate() {
                let pipeline_key = &pipeline_keys[index];
                let pipeline = &self.pipeline_cache[pipeline_key];
                rpass.set_pipeline(pipeline);

                let bg_key = &bg_keys[index];
                let bind_group = &self.bind_group_cache[bg_key];

                let frame_offset = (index * frame_stride) as u32;
                let draw_offset = (index as u64 * self.draw_uniform_stride) as u32;
                rpass.set_bind_group(0, bind_group, &[frame_offset, draw_offset]);

                let vp = &viewports[index];
                let max_dim = target_width.max(target_height) as f32;
                let vp_w = vp.w.clamp(1.0, max_dim);
                let vp_h = vp.h.clamp(1.0, max_dim);
                if vp.x.is_finite() && vp.y.is_finite() && vp_w.is_finite() && vp_h.is_finite() {
                    rpass.set_viewport(vp.x, vp.y, vp_w, vp_h, vp.min_depth, vp.max_depth);
                }

                let sc = &scissors[index];
                let sc_w = sc.w.max(1).min(target_width.saturating_sub(sc.x));
                let sc_h = sc.h.max(1).min(target_height.saturating_sub(sc.y));
                rpass.set_scissor_rect(sc.x, sc.y, sc_w, sc_h);

                rpass.draw(first_vertex..first_vertex + vertex_count, 0..1);
            }
        }

        queue.submit([encoder.finish()]);
    }

    fn ensure_sampler(&mut self, device: &wgpu::Device, key: &SamplerKey) {
        self.sampler_cache.entry(*key).or_insert_with(|| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("gx_sampler"),
                address_mode_u: helpers::map_wrap_mode(key.0),
                address_mode_v: helpers::map_wrap_mode(key.1),
                mag_filter: helpers::map_mag_filter(key.2),
                min_filter: helpers::map_min_filter(key.3),
                ..Default::default()
            })
        });
    }
}

/// Triangulate a [`DrawData`] into GPU vertices.
fn triangulate_draw_data(draw: &DrawData, out: &mut Vec<GpuVertex>) {
    use gecko::flipper::gx::draw::Primitive;

    let verts = &draw.vertices;
    let to_gpu = |v: &gecko::host::DrawVertex| -> GpuVertex {
        GpuVertex {
            position: v.position,
            color: v.color0,
            color1: v.color1,
            normal: v.normal,
            pos_view: v.pos_view,
            tex0: v.texcoords[0],
            tex1: v.texcoords[1],
            tex2: v.texcoords[2],
            tex3: v.texcoords[3],
            tex4: v.texcoords[4],
            tex5: v.texcoords[5],
            tex6: v.texcoords[6],
            tex7: v.texcoords[7],
        }
    };

    match draw.primitive {
        Primitive::Triangles => {
            out.extend(verts.iter().map(to_gpu));
        }
        Primitive::Quads => {
            for quad in verts.chunks(4) {
                if quad.len() < 4 {
                    continue;
                }
                out.push(to_gpu(&quad[0]));
                out.push(to_gpu(&quad[1]));
                out.push(to_gpu(&quad[2]));
                out.push(to_gpu(&quad[0]));
                out.push(to_gpu(&quad[2]));
                out.push(to_gpu(&quad[3]));
            }
        }
        Primitive::TriangleStrip => {
            for i in 2..verts.len() {
                if i % 2 == 0 {
                    out.push(to_gpu(&verts[i - 2]));
                    out.push(to_gpu(&verts[i - 1]));
                    out.push(to_gpu(&verts[i]));
                } else {
                    out.push(to_gpu(&verts[i - 1]));
                    out.push(to_gpu(&verts[i - 2]));
                    out.push(to_gpu(&verts[i]));
                }
            }
        }
        Primitive::TriangleFan => {
            for i in 2..verts.len() {
                out.push(to_gpu(&verts[0]));
                out.push(to_gpu(&verts[i - 1]));
                out.push(to_gpu(&verts[i]));
            }
        }
        _ => {
            tracing::error!(?draw.primitive, "triangulate: skipping unsupported primitive");
        }
    }
}
