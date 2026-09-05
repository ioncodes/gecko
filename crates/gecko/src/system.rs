use crate::HostInput;
use crate::audio::{AudioSink, EmptyAudioSink};
use crate::dvd::DvdInterface;
use crate::flipper::ai::AudioInterface;
use crate::flipper::cp::CommandProcessor;
use crate::flipper::dsp::Dsp;
use crate::flipper::exi::ExternalInterface;
use crate::flipper::gx::GraphicsProcessor;
use crate::flipper::mi::MemoryInterface;
use crate::flipper::pe::PixelEngine;
use crate::flipper::pi::ProcessorInterface;
use crate::flipper::si::SerialInterface;
use crate::flipper::vi::VideoInterface;
#[cfg(feature = "fps-counter")]
use crate::fps::FpsCounter;
use crate::gekko::Gekko;
use crate::hollywood::Hollywood;
#[cfg(feature = "hooks")]
use crate::hooks::{HookFilters, HookFlags, HookState, Host};
use crate::host::{EmptyRenderSink, RenderSink};
use crate::input::InputSink;
use crate::mmio::Mmio;
use crate::scheduler::Scheduler;
use crate::starlet::Starlet;
use image::Executable;

pub type SystemId = u8;

pub const GC: SystemId = 0;
pub const WII: SystemId = 1;

/// This only matters if `jit` feature is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionMode {
    #[default]
    Jit,
    Interpreter,
}

#[cfg(feature = "gx-stats")]
#[derive(Clone, Default)]
struct GxStatsDumpState {
    vsync_count: u64,
    gx: crate::flipper::gx::GxStats,
    render: crate::host::RenderStats,
}

pub struct System<const SYSTEM: SystemId> {
    pub vsync_pending: bool,
    pub vi_present_seen_this_frame: bool,
    pub execution_mode: ExecutionMode,
    pub gekko: Gekko,
    pub scheduler: Scheduler<SYSTEM>,
    pub mmio: Mmio<SYSTEM>,
    pub vi: VideoInterface,
    pub pe: PixelEngine,
    pub pi: ProcessorInterface,
    pub dsp: Dsp,
    pub exi: ExternalInterface,
    pub gx: GraphicsProcessor,
    pub cp: CommandProcessor,
    pub di: DvdInterface,
    pub si: SerialInterface,
    pub ai: AudioInterface,
    pub mi: MemoryInterface,

    // Wii stuff.
    pub starlet: Starlet,
    pub hollywood: Hollywood,

    /// GX dispatches actions here.
    pub render_sink: Box<dyn RenderSink>,

    /// AID DMA pushes 8-frame stereo s16 blocks here.
    pub audio_sink: Box<dyn AudioSink>,

    /// Controller input is sampled from here FOR WII ONLY.
    pub input_sink: Option<Box<dyn InputSink>>,

    #[cfg(feature = "hooks")]
    pub hook_host: Option<Box<dyn Host<SYSTEM> + Send>>,
    #[cfg(feature = "hooks")]
    pub hook_flags: HookFlags,
    #[cfg(feature = "hooks")]
    pub hook_filters: HookFilters,

    #[cfg(feature = "jit")]
    pub jit: Option<Box<crate::gekko::jit::JitEngine<SYSTEM>>>,

    #[cfg(feature = "fps-counter")]
    pub fps_counter: FpsCounter,

    #[cfg(any(feature = "jit-stats", feature = "profile", feature = "gx-stats"))]
    pub heatmap: crate::profile::HeatmapConfig,

    #[cfg(any(feature = "jit-stats", feature = "profile", feature = "gx-stats"))]
    pub vsync_count: u64,

    #[cfg(feature = "gx-stats")]
    gx_stats_dump_state: Option<GxStatsDumpState>,

    #[cfg(feature = "profile")]
    pub pprof_config: Option<crate::profile::PprofConfig>,

    #[cfg(feature = "profile")]
    pub pprof_session: Option<crate::profile::IpSampler>,
}

impl<const SYSTEM: SystemId> System<SYSTEM> {
    pub(crate) fn with_scheduler(entrypoint: u32, scheduler: Scheduler<SYSTEM>) -> Self {
        System {
            vsync_pending: false,
            vi_present_seen_this_frame: false,
            execution_mode: ExecutionMode::default(),
            gekko: Gekko::new(entrypoint),
            scheduler,
            mmio: Mmio::new(),
            vi: VideoInterface::new(),
            pe: PixelEngine::new(),
            pi: ProcessorInterface::new(),
            dsp: Dsp::new(),
            exi: ExternalInterface::dummy(),
            gx: GraphicsProcessor::new(),
            cp: CommandProcessor::new(),
            di: DvdInterface::new(),
            si: SerialInterface::new(),
            ai: AudioInterface::new(),
            mi: MemoryInterface::new(),

            starlet: Starlet::new(),
            hollywood: Hollywood::new(),

            render_sink: Box::new(EmptyRenderSink::default()),
            audio_sink: Box::new(EmptyAudioSink),
            input_sink: None,

            #[cfg(feature = "hooks")]
            hook_host: None,
            #[cfg(feature = "hooks")]
            hook_flags: HookFlags::empty(),
            #[cfg(feature = "hooks")]
            hook_filters: HookFilters::default(),

            #[cfg(feature = "jit")]
            jit: None,

            #[cfg(feature = "fps-counter")]
            fps_counter: FpsCounter::new(),

            #[cfg(any(feature = "jit-stats", feature = "profile", feature = "gx-stats"))]
            heatmap: crate::profile::HeatmapConfig::default(),

            #[cfg(any(feature = "jit-stats", feature = "profile", feature = "gx-stats"))]
            vsync_count: 0,

            #[cfg(feature = "gx-stats")]
            gx_stats_dump_state: None,

            #[cfg(feature = "profile")]
            pprof_config: None,

            #[cfg(feature = "profile")]
            pprof_session: None,
        }
    }

    #[inline(always)]
    pub fn step_cpu(&mut self) {
        if self.gekko.msr.external_interrupt_enable() {
            // Deliver external interrupt when EE=1 and any enabled PI interrupt is pending
            if self.pi.interrupt_pending() {
                self.cause_external_interrupt();
                self.scheduler.cycles += 2;
                return;
            }

            if self.gekko.dec.interrupt_pending() {
                self.cause_decrementer_interrupt();
                self.scheduler.cycles += 2;
                return;
            }
        }

        // CPU pre-hook
        #[cfg(feature = "hooks")]
        if self.hook_flags.contains(HookFlags::CPU_PRE) {
            let pc = self.gekko.pc;
            if self.hook_filters.cpu_pre.matches(pc) {
                if let Some(mut host) = self.hook_host.take() {
                    host.on_cpu_pre(self);
                    self.sync_pending_hook_state(host.as_mut());
                    self.hook_host = Some(host);
                }
            }
        }

        self.gekko.cia = self.gekko.pc;
        self.gekko.nia = self.gekko.cia.wrapping_add(4);
        let instr = crate::gekko::instruction::Instruction(self.mmio.fetch_instruction(self.gekko.cia));
        crate::gekko::dispatch(self, instr);

        // CPU post-hook
        #[cfg(feature = "hooks")]
        if self.hook_flags.contains(HookFlags::CPU_POST) {
            let pc = self.gekko.cia;
            if self.hook_filters.cpu_post.matches(pc) {
                if let Some(mut host) = self.hook_host.take() {
                    host.on_cpu_post(self);
                    self.sync_pending_hook_state(host.as_mut());
                    self.hook_host = Some(host);
                }
            }
        }

        self.gekko.pc = self.gekko.nia;
    }

    /// To JIT or not to JIT, that is the question.
    pub fn set_execution_mode(&mut self, mode: ExecutionMode) {
        self.execution_mode = mode;
        self.gx.execution_mode = mode;
    }

    /// Attach a GameCube memory card on the given EXI channel (0 = slot A,
    /// 1 = slot B), device slot 0. `total_blocks` sets the card size
    /// (256 blocks = "Memory Card 251" / 2 MB card).
    pub fn insert_memory_card(&mut self, channel: usize, path: Option<std::path::PathBuf>, total_blocks: u32) {
        let card = crate::flipper::exi::memcard::ExiMemoryCard::new(path, total_blocks);
        self.exi.attach_device(channel, 0, Box::new(card));
    }

    /// Back the GameCube SRAM (console settings, clock bias, memory card flash
    /// id) with a file so it persists across runs.
    pub fn set_sram_path(&mut self, path: std::path::PathBuf) {
        if let Some(macronix) = self.exi.macronix_mut() {
            macronix.set_sram_path(path);
        }
    }

    /// Drain pending scheduler events, then execute one CPU instruction.
    #[inline(always)]
    pub fn step(&mut self) {
        self.drain_events();
        self.step_cpu();
    }

    pub fn run_until(&mut self, pc: u32, predicate: impl Fn(&Self) -> bool) {
        self.gekko.pc = pc;
        while !predicate(self) {
            self.step();
        }
    }

    #[inline(always)]
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn prepare_frame(&mut self) {
        self.begin_frame();
        crate::flipper::si::refresh_interrupts(self);
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn run_until_vsync(&mut self) {
        self.prepare_frame();
        while !self.vsync_pending {
            self.scheduler.refresh_deadline();
            #[cfg(feature = "jit")]
            if self.execution_mode == ExecutionMode::Jit {
                self.run_until_deadline_jit();
            } else {
                self.run_until_deadline_interp();
            }
            #[cfg(not(feature = "jit"))]
            self.run_until_deadline_interp();
            // Drain all events that are now due
            self.drain_events();
        }

        #[cfg(any(feature = "jit-stats", feature = "profile", feature = "gx-stats"))]
        self.on_vsync_boundary();
    }

    #[cfg(feature = "jit")]
    pub fn load_jit_cache(&mut self, game_id: &str) -> (usize, usize, usize, usize, usize, usize) {
        if self.execution_mode != ExecutionMode::Jit {
            return (0, 0, 0, 0, 0, 0);
        }

        let mut ppc_compiled = 0;
        let mut ppc_skipped = 0;
        let mut dsp_compiled = 0;
        let mut dsp_skipped = 0;
        let mut vtx_compiled = 0;
        let mut vtx_skipped = 0;

        let ppc_path = crate::jit::cache::ppc_cache_path(game_id);
        if let Ok(blocks) = crate::jit::cache::load_ppc_blocks(&ppc_path) {
            tracing::info!(count = blocks.len(), "loaded PPC JIT block cache");

            if self.jit.is_none() {
                self.jit = Some(Box::new(crate::gekko::jit::JitEngine::<SYSTEM>::new()));
            }

            let mut jit = self.jit.take().unwrap();
            let (c, s) = jit.precompile_blocks(self, &blocks);
            ppc_compiled = c;
            ppc_skipped = s;

            self.jit = Some(jit);
        }

        let dsp_path = crate::jit::cache::dsp_cache_path(game_id);
        if let Ok(blocks) = crate::jit::cache::load_dsp_blocks(&dsp_path) {
            tracing::info!(count = blocks.len(), "loaded DSP JIT block cache");

            if self.dsp.jit.is_none() {
                self.dsp.jit = Some(Box::new(crate::flipper::dsp::jit::JitEngine::<SYSTEM>::new()));
            }

            let iram_ptr = self.dsp.iram.as_ptr();
            let irom_ptr = self.dsp.irom.as_ptr();
            let iram_len = self.dsp.iram.len();
            let irom_len = self.dsp.irom.len();
            let iram = unsafe { ::core::slice::from_raw_parts(iram_ptr, iram_len) };
            let irom = unsafe { ::core::slice::from_raw_parts(irom_ptr, irom_len) };

            let (c, s) = self.dsp.jit.as_mut().unwrap().precompile_blocks(iram, irom, &blocks);
            dsp_compiled = c;
            dsp_skipped = s;
        }

        let vtx_path = crate::jit::cache::vtx_cache_path(game_id);
        if let Ok(keys) = crate::jit::cache::load_vtx_keys(&vtx_path) {
            tracing::info!(count = keys.len(), "loaded vertex JIT key cache");
            let (c, s) = self.gx.jit_vtx.precompile_keys(&keys);
            vtx_compiled = c;
            vtx_skipped = s;
        }

        (
            ppc_compiled,
            ppc_skipped,
            dsp_compiled,
            dsp_skipped,
            vtx_compiled,
            vtx_skipped,
        )
    }

    #[cfg(feature = "jit")]
    pub fn save_jit_cache(&self, game_id: &str) -> std::io::Result<(usize, usize, usize)> {
        let cached_system = if SYSTEM == WII {
            crate::jit::cache::CachedSystem::Wii
        } else {
            crate::jit::cache::CachedSystem::Gc
        };

        let mut ppc_count = 0;
        let mut dsp_count = 0;

        if let Some(jit) = self.jit.as_ref() {
            let blocks = jit.cached_blocks();
            ppc_count = blocks.len();
            crate::jit::cache::save_ppc_blocks(&crate::jit::cache::ppc_cache_path(game_id), cached_system, &blocks)?;
        }

        if let Some(jit) = self.dsp.jit.as_ref() {
            let blocks = jit.cached_blocks();
            dsp_count = blocks.len();
            crate::jit::cache::save_dsp_blocks(&crate::jit::cache::dsp_cache_path(game_id), cached_system, &blocks)?;
        }

        let keys = self.gx.jit_vtx.cached_keys();
        let vtx_count = keys.len();
        crate::jit::cache::save_vtx_keys(&crate::jit::cache::vtx_cache_path(game_id), cached_system, &keys)?;

        Ok((ppc_count, dsp_count, vtx_count))
    }

    #[cfg(any(feature = "jit-stats", feature = "profile", feature = "gx-stats"))]
    fn on_vsync_boundary(&mut self) {
        self.vsync_count = self.vsync_count.wrapping_add(1);

        #[cfg(feature = "jit-stats")]
        self.dump_heatmap_if_due();

        #[cfg(feature = "gx-stats")]
        self.dump_gx_stats_if_due();

        #[cfg(feature = "profile")]
        self.tick_pprof_session();
    }

    #[cfg(feature = "gx-stats")]
    fn dump_gx_stats_if_due(&mut self) {
        if !self.heatmap.enabled || self.heatmap.interval_frames == 0 {
            return;
        }

        if self.vsync_count % self.heatmap.interval_frames as u64 != 0 {
            return;
        }

        use std::io::Write;

        let path = self.heatmap.out_dir.join("gx-stats.txt");
        let s = self.gx.stats.clone();
        let avg_draw_ns = if s.draw_calls > 0 {
            s.create_draw_call_ns / s.draw_calls
        } else {
            0
        };

        let render = self.render_sink.render_stats();
        let presents = s.xfb_presents.max(1) as f64;
        let previous = self.gx_stats_dump_state.clone().unwrap_or_default();
        let delta = |current: u64, prior: u64| current.saturating_sub(prior);
        let interval_vsyncs = delta(self.vsync_count, previous.vsync_count);
        let interval_presents = delta(s.xfb_presents, previous.gx.xfb_presents);
        let interval_divisor = interval_presents.max(1) as f64;
        let interval_renderer_draws = delta(render.draws_encoded, previous.render.draws_encoded);
        let interval_potential_draws = delta(render.potential_merged_draws, previous.render.potential_merged_draws);
        let interval_reduction_pct = interval_renderer_draws.saturating_sub(interval_potential_draws) as f64
            / interval_renderer_draws.max(1) as f64
            * 100.0;
        let cumulative_reduction_pct = render.draws_encoded.saturating_sub(render.potential_merged_draws) as f64
            / render.draws_encoded.max(1) as f64
            * 100.0;
        let result = crate::profile::write_file_atomic(&path, |f| {
            writeln!(
                f,
                "vsync_count={}\ndraw_calls={}\nvertices={}\nfifo_bytes={}\ntexture_loads={}\nxfb_presents={}\nbp_writes={}\nxf_writes={}\ncreate_draw_call_ns={}\navg_draw_call_ns={}\nrender_actions_sent={}\nrender_batches_sent={}\nrender_channel_len={}\nrender_channel_cap={}\nrender_channel_high_water={}\nrender_queue_wait_ns={}\nrender_efb_drain_wait_ns={}\nrender_efb_drain_requests={}\nrender_efb_drain_nonempty={}\nrender_efb_writebacks={}\nrender_efb_writeback_cpu_ns={}\nrender_worker_batch_cpu_ns={}\nrender_draws_encoded={}\nrender_draw_passes={}\nrender_pipeline_changes={}\nrender_pipelines_created={}\nrender_pipeline_create_cpu_ns={}\nrender_shader_modules_created={}\nrender_shader_create_cpu_ns={}\nrender_bind_group_sets={}\nrender_bind_groups_created={}\nrender_bind_group_key_changes={}\nrender_frame_uniform_changes={}\nrender_draw_uniform_changes={}\nrender_vertex_stride_changes={}\nrender_potential_merged_draws={}\nrender_viewport_changes={}\nrender_scissor_changes={}\nrender_draw_pass_encode_ns={}\nrender_queue_submits={}\nrender_command_buffers_submitted={}\nrender_queue_submit_cpu_ns={}",
                self.vsync_count,
                s.draw_calls,
                s.vertices,
                s.fifo_bytes,
                s.texture_loads,
                s.xfb_presents,
                s.bp_writes,
                s.xf_writes,
                s.create_draw_call_ns,
                avg_draw_ns,
                render.actions_sent,
                render.batches_sent,
                render.channel_len,
                render.channel_cap,
                render.channel_high_water,
                render.queue_wait_ns,
                render.efb_drain_wait_ns,
                render.efb_drain_requests,
                render.efb_drain_nonempty,
                render.efb_writebacks,
                render.efb_writeback_cpu_ns,
                render.worker_batch_cpu_ns,
                render.draws_encoded,
                render.draw_render_passes,
                render.pipeline_changes,
                render.pipelines_created,
                render.pipeline_create_cpu_ns,
                render.shader_modules_created,
                render.shader_create_cpu_ns,
                render.bind_group_sets,
                render.bind_groups_created,
                render.bind_group_key_changes,
                render.frame_uniform_changes,
                render.draw_uniform_changes,
                render.vertex_stride_changes,
                render.potential_merged_draws,
                render.viewport_changes,
                render.scissor_changes,
                render.draw_pass_encode_ns,
                render.queue_submits,
                render.command_buffers_submitted,
                render.queue_submit_cpu_ns,
            )?;
            writeln!(
                f,
                "\n--- last dump interval ---\nvsyncs={}\nxfb_presents={}\ndraw_calls={}\nvertices={}\nrenderer_draws={}\nactions={}\nbatches={}\ndraws_per_present={:.2}\nvertices_per_present={:.2}\ndraw_passes_per_present={:.2}\npipeline_changes_per_present={:.2}\npipelines_created_per_present={:.2}\npipeline_create_cpu_us_per_present={:.2}\nshader_modules_created_per_present={:.2}\nshader_create_cpu_us_per_present={:.2}\nbind_group_sets_per_present={:.2}\nbind_groups_created_per_present={:.2}\nbind_group_key_changes_per_present={:.2}\nframe_uniform_changes_per_present={:.2}\ndraw_uniform_changes_per_present={:.2}\nvertex_stride_changes_per_present={:.2}\npotential_merged_draws_per_present={:.2}\npotential_draw_reduction_pct={:.2}\nviewport_changes_per_present={:.2}\nscissor_changes_per_present={:.2}\ndraw_pass_encode_cpu_us_per_present={:.2}\nworker_cpu_us_per_present={:.2}\nqueue_wait_us_per_present={:.2}\nefb_drain_wait_us_per_present={:.2}\nefb_drain_requests_per_present={:.2}\nefb_drain_nonempty_per_present={:.2}\nefb_writebacks_per_present={:.2}\nefb_writeback_cpu_us_per_present={:.2}\nqueue_submit_cpu_us_per_present={:.2}",
                interval_vsyncs,
                interval_presents,
                delta(s.draw_calls, previous.gx.draw_calls),
                delta(s.vertices, previous.gx.vertices),
                interval_renderer_draws,
                delta(render.actions_sent, previous.render.actions_sent),
                delta(render.batches_sent, previous.render.batches_sent),
                delta(render.draws_encoded, previous.render.draws_encoded) as f64 / interval_divisor,
                delta(s.vertices, previous.gx.vertices) as f64 / interval_divisor,
                delta(render.draw_render_passes, previous.render.draw_render_passes) as f64 / interval_divisor,
                delta(render.pipeline_changes, previous.render.pipeline_changes) as f64 / interval_divisor,
                delta(render.pipelines_created, previous.render.pipelines_created) as f64 / interval_divisor,
                delta(render.pipeline_create_cpu_ns, previous.render.pipeline_create_cpu_ns,) as f64
                    / interval_divisor
                    / 1_000.0,
                delta(render.shader_modules_created, previous.render.shader_modules_created,) as f64 / interval_divisor,
                delta(render.shader_create_cpu_ns, previous.render.shader_create_cpu_ns) as f64
                    / interval_divisor
                    / 1_000.0,
                delta(render.bind_group_sets, previous.render.bind_group_sets) as f64 / interval_divisor,
                delta(render.bind_groups_created, previous.render.bind_groups_created) as f64 / interval_divisor,
                delta(render.bind_group_key_changes, previous.render.bind_group_key_changes,) as f64 / interval_divisor,
                delta(render.frame_uniform_changes, previous.render.frame_uniform_changes,) as f64 / interval_divisor,
                delta(render.draw_uniform_changes, previous.render.draw_uniform_changes,) as f64 / interval_divisor,
                delta(render.vertex_stride_changes, previous.render.vertex_stride_changes,) as f64 / interval_divisor,
                interval_potential_draws as f64 / interval_divisor,
                interval_reduction_pct,
                delta(render.viewport_changes, previous.render.viewport_changes) as f64 / interval_divisor,
                delta(render.scissor_changes, previous.render.scissor_changes) as f64 / interval_divisor,
                delta(render.draw_pass_encode_ns, previous.render.draw_pass_encode_ns) as f64
                    / interval_divisor
                    / 1_000.0,
                delta(render.worker_batch_cpu_ns, previous.render.worker_batch_cpu_ns) as f64
                    / interval_divisor
                    / 1_000.0,
                delta(render.queue_wait_ns, previous.render.queue_wait_ns) as f64 / interval_divisor / 1_000.0,
                delta(render.efb_drain_wait_ns, previous.render.efb_drain_wait_ns) as f64 / interval_divisor / 1_000.0,
                delta(render.efb_drain_requests, previous.render.efb_drain_requests) as f64 / interval_divisor,
                delta(render.efb_drain_nonempty, previous.render.efb_drain_nonempty) as f64 / interval_divisor,
                delta(render.efb_writebacks, previous.render.efb_writebacks) as f64 / interval_divisor,
                delta(render.efb_writeback_cpu_ns, previous.render.efb_writeback_cpu_ns) as f64
                    / interval_divisor
                    / 1_000.0,
                delta(render.queue_submit_cpu_ns, previous.render.queue_submit_cpu_ns) as f64
                    / interval_divisor
                    / 1_000.0,
            )?;
            writeln!(
                f,
                "\n--- renderer averages per XFB present ---\ndraws={:.2}\ndraw_passes={:.2}\npipeline_changes={:.2}\npipelines_created={:.2}\npipeline_create_cpu_us={:.2}\nshader_modules_created={:.2}\nshader_create_cpu_us={:.2}\nbind_group_sets={:.2}\nbind_groups_created={:.2}\nbind_group_key_changes={:.2}\nframe_uniform_changes={:.2}\ndraw_uniform_changes={:.2}\nvertex_stride_changes={:.2}\npotential_merged_draws={:.2}\npotential_draw_reduction_pct={:.2}\nviewport_changes={:.2}\nscissor_changes={:.2}\ndraw_pass_encode_cpu_us={:.2}\nworker_cpu_us={:.2}\nqueue_wait_us={:.2}\nefb_drain_wait_us={:.2}\nefb_drain_requests={:.2}\nefb_drain_nonempty={:.2}\nefb_writebacks={:.2}\nefb_writeback_cpu_us={:.2}\nqueue_submit_cpu_us={:.2}",
                render.draws_encoded as f64 / presents,
                render.draw_render_passes as f64 / presents,
                render.pipeline_changes as f64 / presents,
                render.pipelines_created as f64 / presents,
                render.pipeline_create_cpu_ns as f64 / presents / 1_000.0,
                render.shader_modules_created as f64 / presents,
                render.shader_create_cpu_ns as f64 / presents / 1_000.0,
                render.bind_group_sets as f64 / presents,
                render.bind_groups_created as f64 / presents,
                render.bind_group_key_changes as f64 / presents,
                render.frame_uniform_changes as f64 / presents,
                render.draw_uniform_changes as f64 / presents,
                render.vertex_stride_changes as f64 / presents,
                render.potential_merged_draws as f64 / presents,
                cumulative_reduction_pct,
                render.viewport_changes as f64 / presents,
                render.scissor_changes as f64 / presents,
                render.draw_pass_encode_ns as f64 / presents / 1_000.0,
                render.worker_batch_cpu_ns as f64 / presents / 1_000.0,
                render.queue_wait_ns as f64 / presents / 1_000.0,
                render.efb_drain_wait_ns as f64 / presents / 1_000.0,
                render.efb_drain_requests as f64 / presents,
                render.efb_drain_nonempty as f64 / presents,
                render.efb_writebacks as f64 / presents,
                render.efb_writeback_cpu_ns as f64 / presents / 1_000.0,
                render.queue_submit_cpu_ns as f64 / presents / 1_000.0,
            )?;
            writeln!(f, "\n--- draws by primitive ---")?;

            use crate::flipper::gx::draw::Primitive;

            const VARIANTS: [Primitive; 7] = [
                Primitive::Quads,
                Primitive::Triangles,
                Primitive::TriangleStrip,
                Primitive::TriangleFan,
                Primitive::Lines,
                Primitive::LineStrip,
                Primitive::Points,
            ];

            for p in VARIANTS {
                let count = s.draws_by_primitive[(p as usize) & 0x7];
                writeln!(f, "  {:>16}  {:?}", count, p)?;
            }

            Ok(())
        });

        if let Err(err) = result {
            tracing::warn!(?err, "gx-stats write failed");
        } else {
            self.gx_stats_dump_state = Some(GxStatsDumpState {
                vsync_count: self.vsync_count,
                gx: s,
                render,
            });
        }
    }

    #[cfg(feature = "jit-stats")]
    fn dump_heatmap_if_due(&mut self) {
        if !self.heatmap.enabled || self.heatmap.interval_frames == 0 {
            return;
        }

        if self.vsync_count % self.heatmap.interval_frames as u64 != 0 {
            return;
        }

        #[cfg(feature = "jit")]
        if let Some(jit) = self.jit.as_ref() {
            if let Err(err) = jit.dump_hot_blocks_csv(self.heatmap.top_k, &self.heatmap.ppc_csv_path()) {
                tracing::warn!(?err, "ppc heatmap dump failed");
            }
        }

        if let Some(jit) = self.dsp.jit.as_ref() {
            if let Err(err) = jit.dump_hot_blocks_csv(self.heatmap.top_k, &self.heatmap.dsp_csv_path()) {
                tracing::warn!(?err, "dsp heatmap dump failed");
            }
        }

        #[cfg(feature = "jit")]
        self.dump_idle_skip();
    }

    #[cfg(all(feature = "jit-stats", feature = "jit"))]
    fn dump_idle_skip(&self) {
        use std::io::Write;
        use std::sync::atomic::Ordering;

        let calls = crate::gekko::jit::runtime::IDLE_SKIP_CALLS.load(Ordering::Relaxed);
        let cycles = crate::gekko::jit::runtime::IDLE_SKIP_CYCLES_ADVANCED.load(Ordering::Relaxed);
        let avg = if calls > 0 { cycles as f64 / calls as f64 } else { 0.0 };
        let dsp_suspends = crate::flipper::dsp::DSP_SUSPEND_COUNT.load(Ordering::Relaxed);
        let dsp_wakes = crate::flipper::dsp::DSP_WAKE_COUNT.load(Ordering::Relaxed);

        let drain_calls = crate::flipper::dsp::DSP_DRAIN_CALLS.load(Ordering::Relaxed);
        let drain_steps = crate::flipper::dsp::DSP_DRAIN_STEPS.load(Ordering::Relaxed);
        let drain_blocks = crate::flipper::dsp::DSP_DRAIN_BLOCKS.load(Ordering::Relaxed);
        let drain_nanos = crate::flipper::dsp::DSP_DRAIN_NANOS.load(Ordering::Relaxed);
        let drain_halt = crate::flipper::dsp::DSP_DRAIN_EXIT_HALT.load(Ordering::Relaxed);
        let drain_answered = crate::flipper::dsp::DSP_DRAIN_EXIT_ANSWERED.load(Ordering::Relaxed);
        let drain_wait = crate::flipper::dsp::DSP_DRAIN_EXIT_WAIT.load(Ordering::Relaxed);
        let drain_budget = crate::flipper::dsp::DSP_DRAIN_EXIT_BUDGET.load(Ordering::Relaxed);
        let batch_calls = crate::flipper::dsp::DSP_BATCH_CALLS.load(Ordering::Relaxed);
        let batch_steps = crate::flipper::dsp::DSP_BATCH_STEPS.load(Ordering::Relaxed);
        let batch_nanos = crate::flipper::dsp::DSP_BATCH_NANOS.load(Ordering::Relaxed);

        let drain_avg_steps = if drain_calls > 0 {
            drain_steps as f64 / drain_calls as f64
        } else {
            0.0
        };
        let drain_avg_block = if drain_blocks > 0 {
            drain_steps as f64 / drain_blocks as f64
        } else {
            0.0
        };
        let drain_ns_per_step = if drain_steps > 0 {
            drain_nanos as f64 / drain_steps as f64
        } else {
            0.0
        };
        let batch_ns_per_step = if batch_steps > 0 {
            batch_nanos as f64 / batch_steps as f64
        } else {
            0.0
        };

        let event_breakdown = self.event_breakdown_top_n(20);

        let path = self.heatmap.out_dir.join("idle-skip.txt");
        let result = crate::profile::write_file_atomic(&path, |f| {
            writeln!(
                f,
                "vsync_count={}\nppc_idle_calls={}\nppc_cycles_advanced={}\nppc_avg_advance={:.1}\ndsp_suspends={}\ndsp_wakes={}",
                self.vsync_count, calls, cycles, avg, dsp_suspends, dsp_wakes
            )?;

            writeln!(
                f,
                "dsp_drain_calls={}\ndsp_drain_steps={}\ndsp_drain_blocks={}\ndsp_drain_ms={:.1}\ndsp_drain_avg_steps={:.1}\ndsp_drain_avg_steps_per_block={:.1}\ndsp_drain_ns_per_step={:.1}",
                drain_calls,
                drain_steps,
                drain_blocks,
                drain_nanos as f64 / 1e6,
                drain_avg_steps,
                drain_avg_block,
                drain_ns_per_step
            )?;

            writeln!(
                f,
                "dsp_drain_exit_halt={}\ndsp_drain_exit_answered={}\ndsp_drain_exit_wait={}\ndsp_drain_exit_budget={}",
                drain_halt, drain_answered, drain_wait, drain_budget
            )?;

            writeln!(
                f,
                "dsp_batch_calls={}\ndsp_batch_steps={}\ndsp_batch_ms={:.1}\ndsp_batch_ns_per_step={:.1}",
                batch_calls,
                batch_steps,
                batch_nanos as f64 / 1e6,
                batch_ns_per_step
            )?;

            writeln!(f, "\n--- top scheduler events by fire count ---")?;

            for (name, count) in &event_breakdown {
                writeln!(f, "{:>10}  {}", count, name)?;
            }

            Ok(())
        });
        if let Err(err) = result {
            tracing::warn!(?err, "idle-skip write failed");
        }
    }

    #[cfg(all(feature = "jit-stats", feature = "jit"))]
    fn event_breakdown_top_n(&self, n: usize) -> Vec<(String, u64)> {
        let mut entries: Vec<(String, u64)> = self
            .scheduler
            .event_fire_counts
            .iter()
            .map(|(&handler, &count)| (format!("{handler:?}"), count))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(n);
        entries
    }

    #[cfg(feature = "profile")]
    fn tick_pprof_session(&mut self) {
        if self.pprof_session.is_none() {
            if let Some(cfg) = self.pprof_config.as_ref() {
                if self.vsync_count >= cfg.delay_vsyncs as u64 {
                    let cfg = self.pprof_config.take().unwrap();
                    match crate::profile::IpSampler::start_for_current_thread(cfg.hz, cfg.secs, cfg.out.clone()) {
                        Ok(s) => {
                            tracing::info!(
                                hz = cfg.hz,
                                secs = cfg.secs,
                                out = %cfg.out.display(),
                                "pprof: sampling started",
                            );
                            self.pprof_session = Some(s);
                        }
                        Err(err) => tracing::warn!(?err, "failed to start pprof sampler"),
                    }
                }
            }
        }

        let expired = self.pprof_session.as_ref().is_some_and(|s| s.expired());
        if expired {
            let session = self.pprof_session.take().unwrap();
            match session.finish() {
                Ok(path) => tracing::info!("pprof samples written to {}", path.display()),
                Err(err) => tracing::warn!(?err, "pprof sample dump failed"),
            }
        }
    }

    #[inline(always)]
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn run_until_deadline_interp(&mut self) {
        while self.scheduler.cycles < self.scheduler.next_deadline() {
            self.step_cpu();
        }
    }

    /// JIT inner loop: runs compiled blocks back-to-back until
    /// `scheduler.cycles >= next_deadline`. Interrupts are checked at block
    /// boundaries.
    #[cfg(feature = "jit")]
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    fn run_until_deadline_jit(&mut self) {
        let mut jit = match self.jit.take() {
            Some(jit) => jit,
            None => Box::new(crate::gekko::jit::JitEngine::<SYSTEM>::new()),
        };

        while self.scheduler.cycles < self.scheduler.next_deadline() {
            if self.gekko.msr.external_interrupt_enable() {
                if self.pi.interrupt_pending() {
                    self.cause_external_interrupt();
                    self.scheduler.cycles += 2;
                    continue;
                }

                if self.gekko.dec.interrupt_pending() {
                    self.cause_decrementer_interrupt();
                    self.scheduler.cycles += 2;
                    continue;
                }
            }

            jit.run_block(self);

            if self.mmio.jit_dirty != 0 {
                jit.drain_scratch.extend(self.mmio.pending_icbi.drain());
                while let Some(line) = jit.drain_scratch.pop() {
                    jit.invalidate_line(&mut self.mmio, line);
                }
                self.mmio.jit_dirty = 0;
            }
        }

        self.jit = Some(jit);
    }

    pub fn frame_size(&self) -> (usize, usize) {
        let fmt = self.vi.dcr.video_format();
        (fmt.columns(), fmt.lines())
    }

    pub fn set_input_sink(&mut self, sink: Box<dyn InputSink>) {
        self.input_sink = Some(sink);
    }

    pub fn sample_host_input(&mut self) {
        let Some(sink) = self.input_sink.as_mut() else {
            return;
        };

        let input = sink.sample();

        self.apply_host_input(&input);

        let rumble = if SYSTEM == WII {
            self.starlet.wiimote_rumble()
        } else {
            self.si.rumble(0)
        };

        if let Some(sink) = self.input_sink.as_mut() {
            sink.set_rumble(0, rumble);
        }
    }

    pub fn apply_host_input(&mut self, input: &HostInput) {
        match input {
            HostInput::Gc(pad) if SYSTEM == GC => {
                self.si.pad_state[0] = *pad;
            }
            HostInput::Wii {
                wiimote_buttons,
                wiimote_shake,
                nunchuk_buttons,
                nunchuk_stick_x,
                nunchuk_stick_y,
                ir_pointer,
                accel,
            } if SYSTEM == WII => {
                self.starlet.set_wiimote_buttons(*wiimote_buttons);
                self.starlet.set_wiimote_shake(*wiimote_shake);
                self.starlet
                    .set_nunchuk(*nunchuk_buttons, *nunchuk_stick_x, *nunchuk_stick_y);
                self.starlet.set_ir_pointer(*ir_pointer);
                self.starlet.set_wiimote_accel(*accel);
            }
            _ => unreachable!("invalid host input for system"),
        }
    }

    #[inline(always)]
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn begin_frame(&mut self) {
        self.vsync_pending = false;
        self.si.update_polling();
    }

    #[inline(always)]
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub fn drain_events(&mut self) {
        while let Some(handler) = self.scheduler.poll() {
            handler.resolve::<SYSTEM>()(self);
        }
    }

    pub fn load_image(&mut self, exe: &impl Executable) {
        let data = exe.data();

        // Copy TEXT sections to memory
        for section in exe.text_sections() {
            for i in 0..section.size {
                let addr = section.vaddr + i;
                let value = data[(section.offset + i) as usize];
                self.mmio.virt_write_u8(addr, value);
            }
        }

        // Copy DATA sections to memory
        for section in exe.data_sections() {
            for i in 0..section.size {
                let addr = section.vaddr + i;
                let value = data[(section.offset + i) as usize];
                self.mmio.virt_write_u8(addr, value);
            }
        }

        // Zero out the BSS section
        let (bss_start, bss_size) = exe.bss();
        for i in 0..bss_size {
            let addr = bss_start + i;
            self.mmio.virt_write_u8(addr, 0);
        }
    }

    #[cfg(feature = "hooks")]
    #[inline(always)]
    pub fn apply_hook_state(&mut self, state: HookState) {
        self.hook_flags = state.flags;
        self.hook_filters = state.filters;
    }

    #[cfg(feature = "hooks")]
    #[inline(always)]
    pub fn sync_pending_hook_state(&mut self, host: &mut dyn Host<SYSTEM>) {
        #[cfg(feature = "hooks-mut-traps")]
        match host.take_pending_hook_state() {
            Ok(Some(state)) => self.apply_hook_state(state),
            Ok(None) => {}
            Err(err) => tracing::error!(target: "script", error = %err, "failed to refresh script traps"),
        }

        #[cfg(not(feature = "hooks-mut-traps"))]
        let _ = host;
    }

    #[cfg(feature = "hooks")]
    pub fn set_hook_host(&mut self, host: Box<dyn Host<SYSTEM> + Send>) {
        self.apply_hook_state(host.hook_state());
        self.hook_host = Some(host);
    }

    #[cfg(feature = "hooks")]
    pub fn has_hook_host(&self) -> bool {
        self.hook_host.is_some()
    }

    #[cfg(not(feature = "hooks"))]
    pub fn has_hook_host(&self) -> bool {
        false
    }

    #[cfg(all(feature = "hooks", feature = "hooks-mut-traps"))]
    pub fn refresh_hook_traps(&mut self) -> Result<(), String> {
        let Some(mut host) = self.hook_host.take() else {
            return Ok(());
        };

        let refresh_result = host.force_refresh_traps();
        match refresh_result {
            Ok(state) => {
                self.apply_hook_state(state);
                self.hook_host = Some(host);
                Ok(())
            }
            Err(err) => {
                self.hook_host = Some(host);
                Err(err)
            }
        }
    }
}
