use crate::pipeline::{FullPipelineKey, PipelineKey, UberPipelineKey};
use crate::shader_specialization::{self, ShaderKey, ShaderSpecializationKey};
use crate::{
    BindGroupCacheKey, DRAW_UNIFORMS_SIZE, DrawUniforms, FRAME_UNIFORMS_SIZE, FrameUniforms, GxRenderer, SamplerKey,
    helpers,
};
use gecko::flipper::gx::draw::Primitive;
use gecko::flipper::gx::regs::{CullMode, MagFilter, MinFilter, WrapMode};
use gecko::host::{DrawData, DrawState, GxAction};
use glam::{Mat4, UVec4, Vec4};
#[cfg(feature = "gx-stats")]
use std::sync::atomic::Ordering;

fn draw_active_slot_mask(draw: &DrawState) -> u8 {
    let mut mask: u8 = 0;

    let num_stages = (draw.num_tev_stages as usize).min(draw.tev_orders.len());
    for i in 0..num_stages {
        let order = draw.tev_orders[i];
        let tex_enabled = ((order >> 6) & 1) != 0;
        if tex_enabled {
            mask |= 1 << (order & 7);
        }
    }

    let num_ind = (draw.num_indirect_stages as usize).min(4);
    for i in 0..num_ind {
        let texmap = (draw.indirect_refs >> (i * 6)) & 7;
        mask |= 1 << texmap;
    }

    mask
}

fn cull_all_suppresses_draw(cull_mode: CullMode, primitive: Primitive) -> bool {
    cull_mode == CullMode::All
        && matches!(
            primitive,
            Primitive::Quads | Primitive::Triangles | Primitive::TriangleStrip | Primitive::TriangleFan
        )
}

fn trim_bg_key(mut key: BindGroupCacheKey, active: u8) -> BindGroupCacheKey {
    for slot in 0..8 {
        if (active >> slot) & 1 == 0 {
            key.tex_keys[slot] = None;
            key.sampler_keys[slot] = None;
        }
    }
    key
}

fn pack_u32_slice_to_uvec4x4(data: &[u32]) -> [UVec4; 4] {
    let get = |i: usize| if i < data.len() { data[i] } else { 0 };
    [
        UVec4::new(get(0), get(1), get(2), get(3)),
        UVec4::new(get(4), get(5), get(6), get(7)),
        UVec4::new(get(8), get(9), get(10), get(11)),
        UVec4::new(get(12), get(13), get(14), get(15)),
    ]
}

fn build_frame_uniform(draw: &DrawState, alpha_cmp: gecko::flipper::gx::regs::AlphaCompare) -> FrameUniforms {
    FrameUniforms {
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
        ztex_bias: draw.ztex_bias,
        ztex_type: draw.ztex_type as u32,
        ztex_op: draw.ztex_op as u32,
        _pad2: 0,
    }
}

impl GxRenderer {
    fn current_pipeline_key(&self, ztex: bool) -> PipelineKey {
        let blend = self.current_blend_mode;
        let zmode = self.current_zmode;
        PipelineKey {
            blend_enable: blend.blend_enable(),
            src_factor: blend.src_factor(),
            dst_factor: blend.dst_factor(),
            subtract: blend.subtract(),
            logic_op_enable: blend.logic_op_enable(),
            logic_op: blend.logic_op(),
            z_enable: zmode.enable(),
            z_func: zmode.func(),
            z_write: zmode.update_enable(),
            color_update: blend.color_update(),
            alpha_update: blend.alpha_update(),
            cull_mode: self.current_cull_mode,
            ztex,
        }
    }

    /// Build a bind-group cache key from the current tracked textures.
    fn current_bind_group_key(&self) -> BindGroupCacheKey {
        BindGroupCacheKey {
            tex_keys: self.current_texture_ids,
            sampler_keys: self.current_sampler_keys,
        }
    }

    /// Append `draw`'s vertices and indices onto the draw already recorded at
    /// `previous`, rebasing the indices onto the merged vertex range.
    fn merge_into_previous(&mut self, previous: usize, draw: &DrawData) {
        let vertex_bias = self.scratch_draws[previous].vertex_count;
        let (_, additional_indices) = self::emit_draw_indices(draw, &mut self.scratch_indices, vertex_bias);

        let merged = &mut self.scratch_draws[previous];
        merged.vertex_count += draw.vertex_count;
        merged.index_count += additional_indices;

        #[cfg(feature = "renderdoc-capture")]
        {
            self.draw_primitives[previous] = Primitive::Triangles;
        }
    }

    /// Replace the renderer's vertex scratch with `new_scratch`, returning the
    /// previous buffer. For sinks (like web) that decouple vertex appending
    /// from action processing vertices are collected in a shared `Vec`
    /// while actions are queued, then both are drained together on the main
    /// thread, at which point the drained scratch must become the renderer's
    /// `scratch_vertices` so each `Draw`'s `base_vertex` indexes correctly.
    pub fn replace_vertex_scratch(
        &mut self,
        new_scratch: Vec<gecko::host::DrawVertex>,
    ) -> Vec<gecko::host::DrawVertex> {
        std::mem::replace(&mut self.scratch_vertices, new_scratch)
    }

    /// Wrapper around [`Self::process_action`] for sinks that wrap a
    /// `GxRenderer` in a `Mutex` (and therefore can't expose this renderer's
    /// `scratch_vertices` directly to the gecko side via the `RenderSink`
    /// trait). Maintains the invariant that `external_scratch` and
    /// `self.scratch_vertices` stay length-synced across `exec` calls: any
    /// new vertices appended to `external_scratch` since the last sync are
    /// brought into `self.scratch_vertices` before processing the action,
    /// and if `process_action` triggers a flush that clears the renderer's
    /// scratch, `external_scratch` is truncated to match.
    pub fn process_action_with_external_scratch(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        action: &GxAction,
        external_scratch: &mut Vec<gecko::host::DrawVertex>,
    ) {
        if external_scratch.len() > self.scratch_vertices.len() {
            let start = self.scratch_vertices.len();
            self.scratch_vertices.extend_from_slice(&external_scratch[start..]);
        }
        self.process_action(device, queue, action);
        if self.scratch_vertices.len() < external_scratch.len() {
            external_scratch.truncate(self.scratch_vertices.len());
        }
    }

    /// Process a single [`GxAction`]. Called on the render worker by the
    /// backend-wgpu sink. Draws are accumulated and flushed lazily when a
    /// non-draw action arrives.
    pub fn process_action(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, action: &GxAction) {
        let draw_stride = self.draw_uniform_stride as usize;
        let draw_struct_size = DRAW_UNIFORMS_SIZE.get() as usize;
        let frame_stride = self.frame_stride;
        let frame_struct_size = FRAME_UNIFORMS_SIZE.get() as usize;

        if !matches!(action, GxAction::Draw(_)) {
            self.draw_fast_path_compatible = false;
        }

        match action {
            GxAction::SetProjection { matrix, is_perspective } => {
                self.current_projection = Mat4::from_cols_array_2d(matrix);
                self.current_projection_perspective = *is_perspective;
            }
            GxAction::SetFreelook { matrix, enabled } => {
                self.freelook = Mat4::from_cols_array_2d(matrix);
                self.freelook_enabled = *enabled;
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
                let tid = *id;
                let copy_size = wgpu::Extent3d {
                    width: *width,
                    height: *height,
                    depth_or_array_layers: 1,
                };
                let direct_copy_layout = wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(*width * 4),
                    rows_per_image: None,
                };

                let keep_cached = self
                    .efb_copy_cache
                    .get(&tid.ram_addr)
                    .is_some_and(|e| e.matches(*fmt, *width, *height));
                if !keep_cached && let Some(entry) = self.efb_copy_cache.remove(&tid.ram_addr) {
                    self.return_to_pool(entry.texture, entry.view);
                }

                if let Some((_, cached_tex, _)) = self.texture_cache.get(&tid) {
                    let size = cached_tex.size();
                    if size.width == *width && size.height == *height {
                        let cached_tex = cached_tex.clone();
                        let staged = self.stage_texture_upload(device, &cached_tex, rgba, *width, *height);

                        if !staged {
                            queue.write_texture(cached_tex.as_image_copy(), rgba, direct_copy_layout, copy_size);
                        }

                        if let Some((cached_fmt, _, _)) = self.texture_cache.get_mut(&tid) {
                            *cached_fmt = *fmt;
                        }

                        return;
                    }
                }

                let pooled = self.texture_pool.get_mut(&(*width, *height)).and_then(|v| v.pop());

                let tex = pooled.unwrap_or_else(|| {
                    let texture_label = format!(
                        "gx_tex addr={:#010x}/{:08x} fmt={:?} size={}x{}",
                        id.ram_addr, id.variant, *fmt, *width, *height
                    );
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: Some(&texture_label),
                        size: copy_size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_DST
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    })
                });

                let staged = self.stage_texture_upload(device, &tex, rgba, *width, *height);
                if !staged {
                    queue.write_texture(tex.as_image_copy(), rgba, direct_copy_layout, copy_size);
                }
                let view = tex.create_view(&Default::default());

                // Cached bind groups still hold the old TextureView for this
                // key. Drop them so they get rebuilt against the new one.
                self.bind_group_cache
                    .retain(|key, _| !key.tex_keys.iter().any(|k| *k == Some(tid)));

                let prior = self.texture_cache.insert(tid, (*fmt, tex, view));
                if let Some((_, old_tex, _)) = prior {
                    self.return_load_texture_to_pool(old_tex);
                }
            }
            GxAction::InvalidateCaches => {
                self.flush_pending_draws(device, queue);
                self.submit_pending(queue);
                self.texture_cache.clear();
                let drained: Vec<_> = self.efb_copy_cache.drain().map(|(_, e)| (e.texture, e.view)).collect();
                for (tex, view) in drained {
                    self.return_to_pool(tex, view);
                }
                self.bind_group_cache.clear();
                self.invalidate_pipeline_caches();
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
                let state_changed = draw.state.is_some();
                if let Some(state) = draw.state.as_ref() {
                    self.current_draw_state.clone_from(state);
                    let shader_key = ShaderKey::from_draw(state, self.current_alpha_compare);
                    self.current_shader_key = Some(shader_key);
                    self.current_specialization_key =
                        ShaderSpecializationKey::from_draw(state, self.current_alpha_compare, shader_key);
                    self.current_active_texture_mask = draw_active_slot_mask(state);
                }

                if cull_all_suppresses_draw(self.current_cull_mode, draw.primitive) {
                    self.last_frame_uniform_index = None;
                    self.draw_fast_path_compatible = false;
                    return;
                }

                if !draw_emits_triangles(draw) {
                    tracing::warn!("draw call produced zero vertices, skipping");
                    self.draw_fast_path_compatible = false;
                    return;
                }

                if !state_changed
                    && self.draw_fast_path_compatible
                    && let Some(previous) = self.scratch_draws.len().checked_sub(1)
                    && let Some(shader_key) = self.current_shader_key
                {
                    let stride = crate::packed_vertex_stride(shader_key.active_texcoords);
                    let previous_draw = self.scratch_draws[previous];
                    if previous_draw.src_vertex_index + previous_draw.vertex_count == draw.base_vertex
                        && previous_draw.packed_vertex_stride == stride
                    {
                        self.merge_into_previous(previous, draw);
                        return;
                    }
                }

                let state = &self.current_draw_state;

                let shader_key = self
                    .current_shader_key
                    .unwrap_or_else(|| ShaderKey::from_draw(state, self.current_alpha_compare));
                let specialization = if self.current_shader_key.is_some() {
                    self.current_specialization_key
                } else {
                    ShaderSpecializationKey::from_draw(state, self.current_alpha_compare, shader_key)
                };
                let ztex_enabled = state.ztex_op != 0;
                let active = if self.current_shader_key.is_some() {
                    self.current_active_texture_mask
                } else {
                    draw_active_slot_mask(state)
                };
                let num_indirect_stages = state.num_indirect_stages;
                let next_frame_uniform = (state_changed || self.last_frame_uniform_index.is_none())
                    .then(|| build_frame_uniform(state, self.current_alpha_compare));
                if !self.shader_cache.contains_key(&shader_key) {
                    #[cfg(feature = "gx-stats")]
                    let shader_started = std::time::Instant::now();
                    let wgsl = shader_specialization::compile_variant(shader_key);
                    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some(&format!("gx_shader_{shader_key:?}")),
                        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
                    });
                    self.shader_cache.insert(shader_key, module);
                    #[cfg(feature = "gx-stats")]
                    {
                        self.stats.shader_modules_created.fetch_add(1, Ordering::Relaxed);
                        self.stats
                            .shader_create_cpu_ns
                            .fetch_add(shader_started.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    }
                    tracing::info!(?shader_key, "compiled specialized shader variant");
                }
                let pipeline_key = self.current_pipeline_key(ztex_enabled);
                let full_key = FullPipelineKey {
                    shader: shader_key,
                    specialization,
                    fixed: pipeline_key,
                };
                if !self.pipeline_cache.contains_key(&full_key) {
                    let uber_key = UberPipelineKey::from(full_key);
                    if !self.uber_pipeline_cache.contains_key(&uber_key) {
                        #[cfg(feature = "gx-stats")]
                        let pipeline_started = std::time::Instant::now();
                        let module = &self.shader_cache[&shader_key];
                        let pipeline = crate::pipeline::create_uber_pipeline(
                            device,
                            &self.pipeline_layout,
                            self.surface_format,
                            module,
                            &uber_key,
                        );
                        self.uber_pipeline_cache.insert(uber_key, pipeline);
                        #[cfg(feature = "gx-stats")]
                        {
                            self.stats.pipelines_created.fetch_add(1, Ordering::Relaxed);
                            self.stats
                                .pipeline_create_cpu_ns
                                .fetch_add(pipeline_started.elapsed().as_nanos() as u64, Ordering::Relaxed);
                        }
                    }
                    self.request_specialized_pipeline(full_key);
                }

                let first_vertex = draw.base_vertex;
                let vertex_count = draw.vertex_count;

                // Build per-draw uniform using tracked projection. The
                // freecam view transform only applies to perspective draws
                // so 2D/HUD passes stay glued to the screen.
                let mvp = if self.freelook_enabled && self.current_projection_perspective {
                    self.current_projection * self.freelook
                } else {
                    self.current_projection
                };
                let mut tex_dims = [UVec4::splat(1); 4];
                if num_indirect_stages > 0 {
                    for slot in 0..8 {
                        if (active >> slot) & 1 == 0 {
                            continue;
                        }
                        let Some(tid) = &self.current_texture_ids[slot] else {
                            continue;
                        };
                        let Some((_, tex, _)) = self.texture_cache.get(tid) else {
                            continue;
                        };

                        let size = tex.size();
                        let c = (slot % 2) * 2;
                        tex_dims[slot / 2][c] = size.width;
                        tex_dims[slot / 2][c + 1] = size.height;
                    }
                }

                let draw_uniform = DrawUniforms { mvp, tex_dims };
                let draw_uniform_bytes = bytemuck::bytes_of(&draw_uniform);
                let bg_key = trim_bg_key(self.current_bind_group_key(), active);
                let stride = crate::packed_vertex_stride(shader_key.active_texcoords);

                // Build a fresh FrameUniforms slot only when the producer
                // signaled state changed since the last draw or this is the
                // first draw of the flush.
                let frame_bytes_len_before = self.frame_uniform_bytes.len();
                let mut frame_was_pushed = false;
                let frame_idx = if let Some(frame_uniform) = next_frame_uniform {
                    let fstart = self.frame_uniform_bytes.len();
                    self.frame_uniform_bytes.resize(fstart + frame_stride, 0);
                    self.frame_uniform_bytes[fstart..fstart + frame_struct_size]
                        .copy_from_slice(bytemuck::bytes_of(&frame_uniform));
                    let idx = (fstart / frame_stride) as u32;
                    self.last_frame_uniform_index = Some(idx);
                    frame_was_pushed = true;
                    idx
                } else {
                    self.last_frame_uniform_index.unwrap()
                };

                let previous_index = self.scratch_draws.len().checked_sub(1);
                let can_merge = previous_index.is_some_and(|previous| {
                    let previous_draw = self.scratch_draws[previous];
                    let previous_draw_start = previous * draw_stride;
                    previous_draw.src_vertex_index + previous_draw.vertex_count == first_vertex
                        && previous_draw.packed_vertex_stride == stride
                        && self.draw_pipeline_keys[previous] == full_key
                        && self.draw_bg_keys[previous] == bg_key
                        && self.draw_viewports[previous] == self.current_viewport
                        && self.draw_scissors[previous] == self.current_scissor
                        && uniform_slots_equal(
                            &self.frame_uniform_bytes,
                            frame_stride,
                            frame_struct_size,
                            self.draw_frame_indices[previous],
                            frame_idx,
                        )
                        && self.scratch_uniform_bytes[previous_draw_start..previous_draw_start + draw_struct_size]
                            == *draw_uniform_bytes
                });

                if let Some(previous) = previous_index.filter(|_| can_merge) {
                    let previous_frame_idx = self.draw_frame_indices[previous];
                    if frame_was_pushed {
                        self.frame_uniform_bytes.truncate(frame_bytes_len_before);
                        self.last_frame_uniform_index = Some(previous_frame_idx);
                        frame_was_pushed = false;
                    }

                    self.merge_into_previous(previous, draw);
                } else {
                    let first_index = self.scratch_indices.len() as u32;
                    let (_, index_count) = emit_draw_indices(draw, &mut self.scratch_indices, 0);

                    let start = self.scratch_draws.len() * draw_stride;
                    self.scratch_uniform_bytes.resize(start + draw_stride, 0);
                    self.scratch_uniform_bytes[start..start + draw_struct_size].copy_from_slice(draw_uniform_bytes);

                    self.draw_pipeline_keys.push(full_key);
                    self.draw_bg_keys.push(bg_key);
                    self.draw_viewports.push(self.current_viewport);
                    self.draw_scissors.push(self.current_scissor);
                    self.draw_frame_indices.push(frame_idx);
                    #[cfg(feature = "renderdoc-capture")]
                    self.draw_primitives.push(draw.primitive);

                    let packed_byte_offset = self
                        .scratch_draws
                        .last()
                        .map(|d| d.packed_vertex_byte_offset + d.vertex_count * d.packed_vertex_stride)
                        .unwrap_or(0);
                    self.scratch_draws.push(crate::DrawRecord {
                        src_vertex_index: first_vertex,
                        vertex_count,
                        first_index,
                        index_count,
                        packed_vertex_byte_offset: packed_byte_offset,
                        packed_vertex_stride: stride,
                    });
                }

                debug_assert!(!frame_was_pushed || self.last_frame_uniform_index == Some(frame_idx));
                self.draw_fast_path_compatible = true;
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

            GxAction::PresentRawXfb { width, height, pixels } => {
                self.flush_pending_draws(device, queue);
                self.execute_present_raw_xfb(device, queue, *width, *height, pixels);
            }

            GxAction::CopyEfbToTexture {
                dest_addr,
                src_x,
                src_y,
                src_w,
                src_h,
                copy_format: raw_copy_format,
                mipmap,
                stride,
                clear,
                clear_color,
                clear_z,
                color_update,
                alpha_update,
                z_update,
                alpha_supported,
                depth_copy,
                is_intensity,
            } => {
                self.flush_pending_draws(device, queue);
                self.execute_copy_efb_to_texture(
                    device,
                    queue,
                    *dest_addr,
                    *src_x,
                    *src_y,
                    *src_w,
                    *src_h,
                    *raw_copy_format,
                    *mipmap,
                    *stride,
                    *depth_copy,
                    *is_intensity,
                );

                if *clear {
                    self.clear_efb_region(
                        device,
                        queue,
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
        self.poll_compiled_pipelines();
        if self.scratch_draws.is_empty() {
            return;
        }

        debug_assert_eq!(self.scratch_draws.len(), self.draw_pipeline_keys.len());
        debug_assert_eq!(self.scratch_draws.len(), self.draw_bg_keys.len());
        debug_assert_eq!(self.scratch_draws.len(), self.draw_viewports.len());
        debug_assert_eq!(self.scratch_draws.len(), self.draw_scissors.len());
        debug_assert_eq!(self.scratch_draws.len(), self.draw_frame_indices.len());
        debug_assert_eq!(
            self.scratch_uniform_bytes.len(),
            self.scratch_draws.len() * self.draw_uniform_stride as usize
        );

        if self.draw_bufs_write_pending {
            self.submit_pending(queue);
        }

        let fub = std::mem::take(&mut self.frame_uniform_bytes);
        self.upload_buffers(device, queue, &fub);
        self.frame_uniform_bytes = fub;

        self.execute_action_render_pass(device);

        self.scratch_vertices.clear();
        self.scratch_indices.clear();
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

    fn execute_action_render_pass(&mut self, device: &wgpu::Device) {
        #[cfg(feature = "gx-stats")]
        let encode_started = std::time::Instant::now();
        #[cfg(feature = "gx-stats")]
        let mut pipeline_changes = 0u64;
        #[cfg(feature = "gx-stats")]
        let mut viewport_changes = 0u64;
        #[cfg(feature = "gx-stats")]
        let mut scissor_changes = 0u64;
        #[cfg(feature = "gx-stats")]
        let mut bind_groups_created = 0u64;
        #[cfg(feature = "gx-stats")]
        let mut bind_group_key_changes = 0u64;
        #[cfg(feature = "gx-stats")]
        let mut frame_uniform_changes = 0u64;
        #[cfg(feature = "gx-stats")]
        let mut draw_uniform_changes = 0u64;
        #[cfg(feature = "gx-stats")]
        let mut vertex_stride_changes = 0u64;
        #[cfg(feature = "gx-stats")]
        let mut potential_merged_draws = 0u64;

        let target_width = self.scaled(crate::EFB_WIDTH);
        let target_height = self.scaled(crate::EFB_HEIGHT);
        let scale = self.efb_scale as f32;
        let fallback_sampler_key = (WrapMode::Clamp, WrapMode::Clamp, MagFilter::Linear, MinFilter::Linear);
        let frame_stride = self.frame_stride;

        let num_draws = self.draw_bg_keys.len();
        for i in 0..num_draws {
            let bg_key = &self.draw_bg_keys[i];
            if self.bind_group_cache.contains_key(bg_key) {
                continue;
            }

            let new_bg = {
                let mut tex_views: [&wgpu::TextureView; 8] = [&self.fallback_view; 8];
                let mut tex_samplers: [&wgpu::Sampler; 8] = [&self.sampler_cache[&fallback_sampler_key]; 8];

                for slot in 0..8 {
                    if let Some(tid) = &bg_key.tex_keys[slot] {
                        let tex_entry = self.texture_cache.get(tid);
                        let efb_match = tex_entry.and_then(|(tex_fmt, tex_tex, _)| {
                            let size = tex_tex.size();
                            self.efb_copy_cache
                                .get(&tid.ram_addr)
                                .filter(|e| e.matches(*tex_fmt, size.width, size.height))
                                .map(|e| &e.view)
                        });
                        if let Some(view) = efb_match.or_else(|| tex_entry.map(|(_, _, v)| v)) {
                            tex_views[slot] = view;
                        } else {
                            tracing::warn!(
                                slot,
                                ram_addr = format!("{:#010X}", tid.ram_addr),
                                variant = format!("{:08X}", tid.variant),
                                "bind group: texture id missing from cache, using fallback"
                            );
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
                            buffer: &self.draw_buffer,
                            offset: self.draw_buffer_layout.frame_offset,
                            size: Some(FRAME_UNIFORMS_SIZE),
                        }),
                    },
                    1 => wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.draw_buffer,
                            offset: self.draw_buffer_layout.draw_offset,
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
            };
            self.bind_group_cache.insert(self.draw_bg_keys[i].clone(), new_bg);
            #[cfg(feature = "gx-stats")]
            {
                bind_groups_created += 1;
            }
        }

        let mut encoder = self.take_or_create_encoder(device);
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
            let vertex_section_off = self.draw_buffer_layout.vertex_offset;
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

            let index_used = (self.scratch_indices.len() * std::mem::size_of::<u32>()) as u64;
            let index_off = self.draw_buffer_layout.index_offset;
            if index_used > 0 {
                rpass.set_index_buffer(
                    self.draw_buffer.slice(index_off..index_off + index_used),
                    wgpu::IndexFormat::Uint32,
                );
            }

            let mut last_pipeline_ptr: *const wgpu::RenderPipeline = std::ptr::null();
            let mut last_vertex_slice: Option<(u64, u64)> = None;
            let mut last_viewport: Option<(f32, f32, f32, f32, f32, f32)> = None;
            let mut last_scissor: Option<(u32, u32, u32, u32)> = None;
            #[cfg(feature = "gx-stats")]
            let mut last_profile_index: Option<usize> = None;
            #[cfg(feature = "gx-stats")]
            let mut last_profile_pipeline_ptr: *const wgpu::RenderPipeline = std::ptr::null();
            #[cfg(feature = "gx-stats")]
            let mut last_profile_viewport: Option<(f32, f32, f32, f32, f32, f32)> = None;
            #[cfg(feature = "gx-stats")]
            let mut last_profile_scissor: Option<(u32, u32, u32, u32)> = None;
            #[cfg(feature = "gx-stats")]
            let mut last_profile_vertex_end: Option<u64> = None;

            for (index, draw) in self.scratch_draws.iter().copied().enumerate() {
                let crate::DrawRecord {
                    vertex_count,
                    first_index,
                    index_count,
                    packed_vertex_byte_offset,
                    packed_vertex_stride,
                    src_vertex_index: _,
                } = draw;

                let first_vertex = 0u32;
                let vertex_bytes = u64::from(vertex_count) * u64::from(packed_vertex_stride);
                let vertex_slice_start = vertex_section_off + u64::from(packed_vertex_byte_offset);
                let vertex_slice_end = vertex_slice_start + vertex_bytes;
                if last_vertex_slice != Some((vertex_slice_start, vertex_slice_end)) {
                    rpass.set_vertex_buffer(0, self.draw_buffer.slice(vertex_slice_start..vertex_slice_end));
                    last_vertex_slice = Some((vertex_slice_start, vertex_slice_end));
                }

                #[cfg(feature = "renderdoc-capture")]
                {
                    let primitive = self.draw_primitives[index];
                    let draw_label = format!(
                        "GX Primitive Batch {index}: primitive={primitive:?} first_vertex={first_vertex} vertex_count={vertex_count} indices={index_count}"
                    );
                    rpass.push_debug_group(&draw_label);
                }
                let full_key = &self.draw_pipeline_keys[index];
                let pipeline = self
                    .pipeline_cache
                    .get(full_key)
                    .unwrap_or_else(|| &self.uber_pipeline_cache[&UberPipelineKey::from(*full_key)]);
                let pipeline_ptr = pipeline as *const wgpu::RenderPipeline;
                if pipeline_ptr != last_pipeline_ptr {
                    rpass.set_pipeline(pipeline);
                    last_pipeline_ptr = pipeline_ptr;
                    #[cfg(feature = "gx-stats")]
                    {
                        pipeline_changes += 1;
                    }
                }

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
                let vp_x = vp.x * scale;
                let vp_y = vp.y * scale;
                let max_dim = target_width.max(target_height) as f32;
                let vp_w = (vp.w * scale).clamp(1.0, max_dim);
                let vp_h = (vp.h * scale).clamp(1.0, max_dim);

                if vp_x.is_finite() && vp_y.is_finite() && vp_w.is_finite() && vp_h.is_finite() {
                    let mut min_d = vp.min_depth.clamp(0.0, 1.0);
                    let mut max_d = vp.max_depth.clamp(0.0, 1.0);
                    if min_d > max_d {
                        std::mem::swap(&mut min_d, &mut max_d);
                    }
                    let vp_tuple = (vp_x, vp_y, vp_w, vp_h, min_d, max_d);
                    if last_viewport != Some(vp_tuple) {
                        rpass.set_viewport(vp_x, vp_y, vp_w, vp_h, min_d, max_d);
                        last_viewport = Some(vp_tuple);
                        #[cfg(feature = "gx-stats")]
                        {
                            viewport_changes += 1;
                        }
                    }
                } else {
                    tracing::warn!(
                        x = vp_x,
                        y = vp_y,
                        w = vp_w,
                        h = vp_h,
                        "non-finite viewport, skipping set_viewport"
                    );
                }

                let sc = &self.draw_scissors[index];
                let sc_x = self.scaled(sc.x).min(target_width);
                let sc_y = self.scaled(sc.y).min(target_height);
                let sc_w = self.scaled(sc.w).min(target_width - sc_x);
                let sc_h = self.scaled(sc.h).min(target_height - sc_y);
                let sc_tuple = (sc_x, sc_y, sc_w, sc_h);
                if last_scissor != Some(sc_tuple) {
                    rpass.set_scissor_rect(sc_x, sc_y, sc_w, sc_h);
                    last_scissor = Some(sc_tuple);
                    #[cfg(feature = "gx-stats")]
                    {
                        scissor_changes += 1;
                    }
                }

                #[cfg(feature = "gx-stats")]
                {
                    let draw_stride = self.draw_uniform_stride as usize;
                    let draw_size = DRAW_UNIFORMS_SIZE.get() as usize;
                    let frame_size = FRAME_UNIFORMS_SIZE.get() as usize;
                    let draw_start = index * draw_stride;
                    let frame_start = self.draw_frame_indices[index] as usize * frame_stride;

                    let (pipeline_same, bind_group_same, frame_uniform_same, draw_uniform_same, stride_same) =
                        if let Some(previous) = last_profile_index {
                            let previous_draw_start = previous * draw_stride;
                            let previous_frame_start = self.draw_frame_indices[previous] as usize * frame_stride;
                            (
                                pipeline_ptr == last_profile_pipeline_ptr,
                                self.draw_bg_keys[index] == self.draw_bg_keys[previous],
                                self.draw_frame_indices[index] == self.draw_frame_indices[previous]
                                    || self.frame_uniform_bytes[frame_start..frame_start + frame_size]
                                        == self.frame_uniform_bytes
                                            [previous_frame_start..previous_frame_start + frame_size],
                                self.scratch_uniform_bytes[draw_start..draw_start + draw_size]
                                    == self.scratch_uniform_bytes[previous_draw_start..previous_draw_start + draw_size],
                                packed_vertex_stride == self.scratch_draws[previous].packed_vertex_stride,
                            )
                        } else {
                            (false, false, false, false, false)
                        };

                    if !bind_group_same {
                        bind_group_key_changes += 1;
                    }
                    if !frame_uniform_same {
                        frame_uniform_changes += 1;
                    }
                    if !draw_uniform_same {
                        draw_uniform_changes += 1;
                    }
                    if !stride_same {
                        vertex_stride_changes += 1;
                    }

                    let compatible = last_profile_index.is_some()
                        && pipeline_same
                        && bind_group_same
                        && frame_uniform_same
                        && draw_uniform_same
                        && stride_same
                        && last_profile_viewport == last_viewport
                        && last_profile_scissor == Some(sc_tuple)
                        && last_profile_vertex_end == Some(vertex_slice_start);
                    if !compatible {
                        potential_merged_draws += 1;
                    }

                    last_profile_index = Some(index);
                    last_profile_pipeline_ptr = pipeline_ptr;
                    last_profile_viewport = last_viewport;
                    last_profile_scissor = Some(sc_tuple);
                    last_profile_vertex_end = Some(vertex_slice_end);
                }

                #[cfg(feature = "renderdoc-capture")]
                {
                    let raster_marker = format!(
                        "Raster state: viewport=({:.1},{:.1} {:.1}x{:.1}) scissor=({},{} {}x{})",
                        vp.x, vp.y, vp_w, vp_h, sc_x, sc_y, sc_w, sc_h
                    );
                    rpass.insert_debug_marker(&raster_marker);
                }

                if index_count == 0 {
                    rpass.draw(first_vertex..first_vertex + vertex_count, 0..1);
                } else {
                    rpass.draw_indexed(first_index..first_index + index_count, first_vertex as i32, 0..1);
                }

                #[cfg(feature = "renderdoc-capture")]
                rpass.pop_debug_group();
            }
        }

        #[cfg(feature = "renderdoc-capture")]
        encoder.pop_debug_group();

        self.current_encoder = Some(encoder);
        self.draw_bufs_write_pending = true;

        #[cfg(feature = "gx-stats")]
        {
            self.stats.draws_encoded.fetch_add(num_draws as u64, Ordering::Relaxed);
            self.stats.draw_render_passes.fetch_add(1, Ordering::Relaxed);
            self.stats
                .pipeline_changes
                .fetch_add(pipeline_changes, Ordering::Relaxed);
            self.stats
                .bind_group_sets
                .fetch_add(num_draws as u64, Ordering::Relaxed);
            self.stats
                .bind_groups_created
                .fetch_add(bind_groups_created, Ordering::Relaxed);
            self.stats
                .bind_group_key_changes
                .fetch_add(bind_group_key_changes, Ordering::Relaxed);
            self.stats
                .frame_uniform_changes
                .fetch_add(frame_uniform_changes, Ordering::Relaxed);
            self.stats
                .draw_uniform_changes
                .fetch_add(draw_uniform_changes, Ordering::Relaxed);
            self.stats
                .vertex_stride_changes
                .fetch_add(vertex_stride_changes, Ordering::Relaxed);
            self.stats
                .potential_merged_draws
                .fetch_add(potential_merged_draws, Ordering::Relaxed);
            self.stats
                .viewport_changes
                .fetch_add(viewport_changes, Ordering::Relaxed);
            self.stats.scissor_changes.fetch_add(scissor_changes, Ordering::Relaxed);
            self.stats
                .draw_pass_encode_ns
                .fetch_add(encode_started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
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

fn uniform_slots_equal(bytes: &[u8], stride: usize, size: usize, a: u32, b: u32) -> bool {
    if a == b {
        return true;
    }
    let a = a as usize * stride;
    let b = b as usize * stride;
    bytes[a..a + size] == bytes[b..b + size]
}

fn draw_emits_triangles(draw: &DrawData) -> bool {
    use gecko::flipper::gx::draw::Primitive;

    match draw.primitive {
        Primitive::Triangles | Primitive::TriangleStrip | Primitive::TriangleFan => draw.vertex_count >= 3,
        Primitive::Quads => draw.vertex_count >= 4,
        _ => {
            tracing::error!(?draw.primitive, "draw_emits_triangles: skipping unsupported primitive");
            false
        }
    }
}

fn emit_draw_indices(draw: &DrawData, indices_out: &mut Vec<u32>, vertex_bias: u32) -> (u32, u32) {
    use gecko::flipper::gx::draw::Primitive;

    let n = draw.vertex_count;

    match draw.primitive {
        Primitive::Triangles => {
            indices_out.extend((0..n).map(|i| vertex_bias + i));
            (n, n)
        }
        Primitive::Quads => {
            let chunks = n / 4;
            for q in 0..chunks {
                let base = vertex_bias + q * 4;
                indices_out.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            (n, chunks * 6)
        }
        Primitive::TriangleStrip => {
            if n < 3 {
                return (0, 0);
            }

            let tris = n - 2;
            for i in 0..tris {
                if i & 1 == 0 {
                    indices_out.extend_from_slice(&[vertex_bias + i, vertex_bias + i + 1, vertex_bias + i + 2]);
                } else {
                    indices_out.extend_from_slice(&[vertex_bias + i + 1, vertex_bias + i, vertex_bias + i + 2]);
                }
            }
            (n, tris * 3)
        }
        Primitive::TriangleFan => {
            if n < 3 {
                return (0, 0);
            }

            let tris = n - 2;
            for i in 0..tris {
                indices_out.extend_from_slice(&[vertex_bias, vertex_bias + i + 1, vertex_bias + i + 2]);
            }
            (n, tris * 3)
        }
        _ => {
            tracing::error!(?draw.primitive, "emit_draw_indices: skipping unsupported primitive");
            (0, 0)
        }
    }
}
