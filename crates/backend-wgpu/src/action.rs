use crate::pipeline::{FullPipelineKey, PipelineKey};
use crate::shader_specialization::{self, ShaderKey};
use crate::{
    BindGroupCacheKey, DRAW_UNIFORMS_SIZE, DrawUniforms, FRAME_UNIFORMS_SIZE, FrameUniforms, GpuVertex, GxRenderer,
    SamplerKey, helpers,
};
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

    /// Process a single [`GxAction`]. Called by the worker thread for each
    /// action received from the channel. Draws are accumulated and flushed
    /// lazily when a non-draw action arrives.
    pub fn process_action(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, action: &GxAction) {
        let draw_stride = self.draw_uniform_stride as usize;
        let draw_struct_size = DRAW_UNIFORMS_SIZE.get() as usize;
        let frame_stride = self.frame_stride;
        let frame_struct_size = FRAME_UNIFORMS_SIZE.get() as usize;

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
                fmt,
                rgba,
            } => {
                let texture_label = format!(
                    "gx_tex addr={:#010x}/{:08x} fmt={:?} size={}x{}",
                    id.ram_addr, id.variant, *fmt, *width, *height
                );
                let tex = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(&texture_label),
                    size: wgpu::Extent3d {
                        width: *width,
                        height: *height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC,
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

                // Invalidate bind groups that referenced the old entry for
                // this exact cache id (variant-specific).
                let tid = *id;
                self.bind_group_cache
                    .retain(|key, _| !key.tex_keys.iter().any(|k| *k == Some(tid)));

                // A fresh RAM upload means the game wrote this address
                // after a prior EFB copy. Return the stale GPU copy to
                // its pool and let the RAM-decoded entry win. EFB copies
                // are keyed by bare ram_addr (no TLUT variant).
                if let Some((old_tex, old_view)) = self.efb_copy_cache.remove(&tid.ram_addr) {
                    self.return_to_pool(old_tex, old_view);
                }

                self.texture_cache.insert(*id, (*fmt, tex, view));
            }
            GxAction::InvalidateCaches => {
                self.flush_pending_draws(device, queue);
                self.texture_cache.clear();
                let drained: Vec<_> = self.efb_copy_cache.drain().map(|(_, v)| v).collect();
                for (tex, view) in drained {
                    self.return_to_pool(tex, view);
                }
                self.bind_group_cache.clear();
                self.pipeline_cache.clear();
            }
            #[cfg(not(target_arch = "wasm32"))]
            GxAction::DumpTextures { dir } => {
                self.flush_pending_draws(device, queue);
                self.dump_textures(device, queue, dir);
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
                let shader_key = ShaderKey::from_draw(draw, self.current_alpha_compare);
                if !self.shader_cache.contains_key(&shader_key) {
                    let wgsl = shader_specialization::compile_variant(shader_key);
                    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some(&format!("gx_shader_{shader_key:?}")),
                        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
                    });
                    self.shader_cache.insert(shader_key, module);
                    tracing::info!(?shader_key, "compiled specialized shader variant");
                }
                let pipeline_key = self.current_pipeline_key();
                let full_key = FullPipelineKey {
                    shader: shader_key,
                    fixed: pipeline_key,
                };
                if !self.pipeline_cache.contains_key(&full_key) {
                    let module = &self.shader_cache[&shader_key];
                    let pipeline = self.create_pipeline(device, module, &pipeline_key);
                    self.pipeline_cache.insert(full_key, pipeline);
                }

                // Triangulate.
                let prev_len = self.scratch_vertices.len();
                triangulate_draw_data(draw, &mut self.scratch_vertices);
                let added = self.scratch_vertices.len() - prev_len;
                if added == 0 {
                    tracing::warn!("draw call produced zero triangulated vertices, skipping");
                    return;
                }

                // Build per-draw uniform using tracked projection.
                let mvp = self.current_projection;
                let draw_uniform = DrawUniforms { mvp };

                let start = self.scratch_draws.len() * draw_stride;
                self.scratch_uniform_bytes.resize(start + draw_stride, 0);
                self.scratch_uniform_bytes[start..start + draw_struct_size]
                    .copy_from_slice(bytemuck::bytes_of(&draw_uniform));

                // Build a fresh FrameUniforms slot only when the producer
                // signaled state changed since the last draw or this is the
                // first draw of the flush.
                let frame_idx = if draw.frame_dirty || self.last_frame_uniform_index.is_none() {
                    let alpha_cmp = self.current_alpha_compare;
                    let frame_uniform = FrameUniforms {
                        tev_color_regs: draw.tev_color_regs.map(Vec4::from),
                        tev_konst_colors: draw.tev_konst_colors.map(Vec4::from),
                        tev_color_env: pack_u32_slice_to_uvec4x4(&draw.tev_color_env),
                        tev_alpha_env: pack_u32_slice_to_uvec4x4(&draw.tev_alpha_env),
                        tev_orders: pack_u32_slice_to_uvec4x4(&draw.tev_orders),
                        tev_ksel: pack_u32_slice_to_uvec4x4(&draw.tev_ksel),
                        num_tev_stages: draw.num_tev_stages as u32,
                        alpha_ref0: alpha_cmp.ref0() as f32 / 255.0,
                        alpha_ref1: alpha_cmp.ref1() as f32 / 255.0,
                        alpha_comp0: alpha_cmp.comp0() as u32,
                        alpha_comp1: alpha_cmp.comp1() as u32,
                        alpha_op: alpha_cmp.op() as u32,
                        _pad0: [0; 2],
                        indirect_matrices: draw.indirect_matrices.map(glam::IVec4::from),
                        indirect_scales: draw.indirect_scales.map(glam::UVec4::from),
                        indirect_refs: draw.indirect_refs,
                        num_indirect_stages: draw.num_indirect_stages as u32,
                        bump_imask: draw.bump_imask,
                        _pad1: 0,
                        tev_indirect: pack_u32_slice_to_uvec4x4(&draw.tev_indirect),
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

                    let fstart = self.frame_uniform_bytes.len();
                    self.frame_uniform_bytes.resize(fstart + frame_stride, 0);
                    self.frame_uniform_bytes[fstart..fstart + frame_struct_size]
                        .copy_from_slice(bytemuck::bytes_of(&frame_uniform));
                    let idx = (fstart / frame_stride) as u32;
                    self.last_frame_uniform_index = Some(idx);
                    idx
                } else {
                    self.last_frame_uniform_index.unwrap()
                };

                // Snapshot tracked state for this draw.
                self.draw_pipeline_keys.push(full_key);
                self.draw_bg_keys.push(self.current_bind_group_key());
                self.draw_viewports.push(self.current_viewport);
                self.draw_scissors.push(self.current_scissor);
                self.draw_frame_indices.push(frame_idx);
                #[cfg(feature = "renderdoc-capture")]
                self.draw_primitives.push(draw.primitive);

                self.scratch_draws.push((prev_len as u32, added as u32));
            }

            GxAction::CopyXfb {
                id,
                src_x,
                src_y,
                src_w,
                src_h,
                dst_h,
                gamma,
                clear,
                clear_color,
                clear_z,
                color_update,
                alpha_update,
                z_update,
                alpha_supported,
            } => {
                self.flush_pending_draws(device, queue);
                self.execute_copy_xfb(
                    device,
                    queue,
                    *id,
                    *src_x,
                    *src_y,
                    *src_w,
                    *src_h,
                    *dst_h,
                    *gamma,
                    *clear,
                    *clear_color,
                    *clear_z,
                    *color_update,
                    *alpha_update,
                    *z_update,
                    *alpha_supported,
                );
            }

            GxAction::PresentXfb { width, height, parts } => {
                self.flush_pending_draws(device, queue);
                self.execute_present_xfb(device, queue, *width, *height, parts);
            }

            GxAction::CopyEfbToTexture {
                dest_addr,
                src_x,
                src_y,
                src_w,
                src_h,
                copy_format: _copy_format,
                mipmap,
                stride: _stride,
                clear,
                clear_color,
                clear_z,
                color_update,
                alpha_update,
                z_update,
                alpha_supported,
                depth_copy,
            } => {
                self.flush_pending_draws(device, queue);

                // With `efb-writeback`: read the EFB back, encode it in
                // `copy_format`, ship the bytes to the emu thread so they
                // land in `Mmio::ram` at `dest_addr`. Expensive (GPU stall).
                #[cfg(feature = "efb-writeback")]
                self.execute_copy_efb_to_texture(
                    device,
                    queue,
                    *dest_addr,
                    *src_x,
                    *src_y,
                    *src_w,
                    *src_h,
                    *_copy_format,
                    *mipmap,
                    *_stride,
                    *depth_copy,
                    *clear,
                    *clear_color,
                    *clear_z,
                    *color_update,
                    *alpha_update,
                    *z_update,
                    *alpha_supported,
                );

                // Blit or resolve the EFB region into a GPU texture keyed by `dest_addr`.
                if *depth_copy {
                    self.cache_efb_copy_depth(device, queue, *dest_addr, *src_x, *src_y, *src_w, *src_h, *mipmap);
                } else {
                    self.cache_efb_copy_color(device, queue, *dest_addr, *src_x, *src_y, *src_w, *src_h, *mipmap);
                }

                if *clear {
                    self.efb_clear.clear_region_masked(
                        device,
                        queue,
                        &self.efb_msaa_view,
                        &self.efb_view,
                        &self.efb_depth_view,
                        crate::EFB_WIDTH,
                        crate::EFB_HEIGHT,
                        *src_x,
                        *src_y,
                        *src_w,
                        *src_h,
                        *clear_color,
                        *clear_z,
                        *color_update,
                        *alpha_update && *alpha_supported,
                        *z_update,
                    );
                }
            }
        }
    }

    /// Flush accumulated draw calls into a render pass.
    pub fn flush_pending_draws(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.scratch_draws.is_empty() {
            return;
        }
        let fub = std::mem::take(&mut self.frame_uniform_bytes);
        self.upload_buffers(device, queue, &fub);
        self.frame_uniform_bytes = fub;

        self.execute_action_render_pass(device, queue);

        self.scratch_vertices.clear();
        self.scratch_draws.clear();
        self.scratch_uniform_bytes.clear();
        self.frame_uniform_bytes.clear();
        self.draw_pipeline_keys.clear();
        self.draw_bg_keys.clear();
        self.draw_viewports.clear();
        self.draw_scissors.clear();
        self.draw_frame_indices.clear();
        self.last_frame_uniform_index = None;
        #[cfg(feature = "renderdoc-capture")]
        self.draw_primitives.clear();
    }

    fn execute_action_render_pass(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let target_width = crate::EFB_WIDTH;
        let target_height = crate::EFB_HEIGHT;
        let fallback_sampler_key = (WrapMode::Clamp, WrapMode::Clamp, MagFilter::Linear, MinFilter::Linear);
        let frame_stride = self.frame_stride;

        // Pre-build bind groups from tracked texture snapshots.
        for bg_key in &self.draw_bg_keys {
            self.bind_group_cache.entry(bg_key.clone()).or_insert_with(|| {
                let mut tex_views: [&wgpu::TextureView; 8] = [&self.fallback_view; 8];
                let mut tex_samplers: [&wgpu::Sampler; 8] = [&self.sampler_cache[&fallback_sampler_key]; 8];

                for slot in 0..8 {
                    if let Some(tid) = &bg_key.tex_keys[slot] {
                        // EFB copies (keyed by bare ram_addr) win over the
                        // RAM-decoded `texture_cache` (keyed by full TextureKey).
                        let view = self
                            .efb_copy_cache
                            .get(&tid.ram_addr)
                            .map(|(_, v)| v)
                            .or_else(|| self.texture_cache.get(tid).map(|(_, _, v)| v));
                        if let Some(view) = view {
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
                            size: Some(FRAME_UNIFORMS_SIZE),
                        }),
                    },
                    1 => wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.draw_uniform_buffer,
                            offset: 0,
                            size: Some(DRAW_UNIFORMS_SIZE),
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

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gx_draw_flush_encoder"),
        });
        #[cfg(feature = "renderdoc-capture")]
        let group_label = format!(
            "GX FIFO Execution / EFB Rendering: draws={} vertices={}",
            self.scratch_draws.len(),
            self.scratch_vertices.len()
        );
        #[cfg(feature = "renderdoc-capture")]
        encoder.push_debug_group(&group_label);
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
                label: Some("efb_rendering"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: if crate::EFB_SAMPLE_COUNT == 1 {
                        &self.efb_view
                    } else {
                        &self.efb_msaa_view
                    },
                    resolve_target: if crate::EFB_SAMPLE_COUNT == 1 {
                        None
                    } else {
                        Some(&self.efb_view)
                    },
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
            #[cfg(feature = "renderdoc-capture")]
            {
                if needs_initial_clear {
                    rpass.insert_debug_marker("EFB initial clear: color=black depth=1.0");
                } else {
                    rpass.insert_debug_marker("EFB load existing color/depth");
                }
                if crate::EFB_SAMPLE_COUNT == 1 {
                    rpass.insert_debug_marker("EFB target: resolved color, no MSAA resolve");
                } else {
                    rpass.insert_debug_marker("EFB target: MSAA color, resolve to efb_color_resolved");
                }
            }

            for (index, (first_vertex, vertex_count)) in self.scratch_draws.iter().copied().enumerate() {
                #[cfg(feature = "renderdoc-capture")]
                {
                    let primitive = self.draw_primitives[index];
                    let draw_label = format!(
                        "GX Primitive Batch {index}: primitive={primitive:?} first_vertex={first_vertex} vertex_count={vertex_count}"
                    );
                    rpass.push_debug_group(&draw_label);
                }
                let full_key = &self.draw_pipeline_keys[index];
                let pipeline = &self.pipeline_cache[full_key];
                rpass.set_pipeline(pipeline);
                #[cfg(feature = "renderdoc-capture")]
                {
                    let pipeline_marker = format!(
                        "Pipeline: blend={} z={} z_write={} color_update={} alpha_update={} shader={:?}",
                        full_key.fixed.blend_enable,
                        full_key.fixed.z_enable,
                        full_key.fixed.z_write,
                        full_key.fixed.color_update,
                        full_key.fixed.alpha_update,
                        full_key.shader,
                    );
                    rpass.insert_debug_marker(&pipeline_marker);
                }

                let bg_key = &self.draw_bg_keys[index];
                let bind_group = &self.bind_group_cache[bg_key];

                let frame_offset = self.draw_frame_indices[index] * frame_stride as u32;
                let draw_offset = (index as u64 * self.draw_uniform_stride) as u32;
                rpass.set_bind_group(0, bind_group, &[frame_offset, draw_offset]);
                #[cfg(feature = "renderdoc-capture")]
                {
                    let bind_marker = format!("Bind group offsets: frame={frame_offset} draw={draw_offset}");
                    rpass.insert_debug_marker(&bind_marker);
                }

                let vp = &self.draw_viewports[index];
                let max_dim = target_width.max(target_height) as f32;
                let vp_w = vp.w.clamp(1.0, max_dim);
                let vp_h = vp.h.clamp(1.0, max_dim);
                if vp.x.is_finite() && vp.y.is_finite() && vp_w.is_finite() && vp_h.is_finite() {
                    rpass.set_viewport(vp.x, vp.y, vp_w, vp_h, vp.min_depth, vp.max_depth);
                } else {
                    tracing::warn!(
                        x = vp.x,
                        y = vp.y,
                        w = vp_w,
                        h = vp_h,
                        "non-finite viewport, skipping set_viewport"
                    );
                }

                let sc = &self.draw_scissors[index];
                let sc_x = sc.x.min(target_width);
                let sc_y = sc.y.min(target_height);
                let sc_w = sc.w.min(target_width - sc_x);
                let sc_h = sc.h.min(target_height - sc_y);
                rpass.set_scissor_rect(sc_x, sc_y, sc_w, sc_h);
                #[cfg(feature = "renderdoc-capture")]
                {
                    let raster_marker = format!(
                        "Raster state: viewport=({:.1},{:.1} {:.1}x{:.1}) scissor=({},{} {}x{})",
                        vp.x, vp.y, vp_w, vp_h, sc_x, sc_y, sc_w, sc_h
                    );
                    rpass.insert_debug_marker(&raster_marker);
                }

                rpass.draw(first_vertex..first_vertex + vertex_count, 0..1);
                #[cfg(feature = "renderdoc-capture")]
                rpass.pop_debug_group();
            }
        }
        #[cfg(feature = "renderdoc-capture")]
        encoder.pop_debug_group();

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
                    tracing::error!(count = quad.len(), "quad primitive with less than 4 vertices, skipping");
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
