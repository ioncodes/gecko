use gecko::flipper::gx::constants::{BP_LOAD_TLUT1, BP_PRELOAD_MODE, BP_REG_SIZE, CP_REG_SIZE};
use gecko::flipper::gx::{GraphicsProcessor, texture};
use gecko::host::{DrawData, DrawSegment, DrawVertex, GxAction, RenderSink, XfbPart};
use gecko::mmio::{Mmio, RamViewMut};
use gecko::system::SystemId;

const BP_RESTORE_SKIP: &[usize] = &[0x45, 0x47, 0x48, 0x52, BP_PRELOAD_MODE, BP_LOAD_TLUT1, 0xFE];

pub struct PlayerSink {
    inner: Box<dyn RenderSink>,
    xfb_heights: Vec<(u32, u32)>,
    pub inspection: Option<crate::debug::geometry::GeometryCapture>,
    pub depth_request: Option<(crate::debug::geometry::DrawLocation, usize)>,
    next_subdraw: usize,
}

impl PlayerSink {
    pub fn new(inner: Box<dyn RenderSink>) -> Self {
        PlayerSink {
            inner,
            xfb_heights: Vec::new(),
            inspection: None,
            depth_request: None,
            next_subdraw: 0,
        }
    }
}

impl RenderSink for PlayerSink {
    fn capture_efb_depth(&mut self) -> Option<Vec<u32>> {
        self.inner.capture_efb_depth()
    }

    fn inspect_draws(&mut self, gx: &GraphicsProcessor, segments: &[DrawSegment]) {
        let Some(capture) = &mut self.inspection else {
            return;
        };

        let verts = self.inner.vertex_scratch();
        self.next_subdraw = capture.draws.get(&capture.location.offset).map_or(0, Vec::len);

        for seg in segments {
            let first = seg.base_vertex as usize;
            capture.record(gx, seg.primitive, &verts[first..first + seg.vertex_count as usize]);
        }
    }

    fn exec(&mut self, action: GxAction) {
        if let GxAction::Draw(data) = &action {
            if let Some(capture) = &self.inspection
                && self.depth_request == Some((capture.location, self.next_subdraw))
                && let Some(draw) = capture
                    .draws
                    .get(&capture.location.offset)
                    .and_then(|v| v.get(self.next_subdraw))
            {
                let _ = draw.depth_before.set(self.inner.capture_efb_depth());
            }
            self.next_subdraw += data.segments.len();
        }

        if let Some(capture) = &mut self.inspection {
            capture.action(&action);
        }

        if let GxAction::CopyXfb { id, dst_h, .. } = &action {
            self.xfb_heights.push((*id, *dst_h));
        }

        self.inner.exec(action);
    }

    fn vertex_scratch(&mut self) -> &mut Vec<DrawVertex> {
        self.inner.vertex_scratch()
    }

    fn flush_efb_copies(&mut self, ram: &mut RamViewMut<'_>) {
        self.inner.flush_efb_copies(ram);
    }

    fn reset_efb(&mut self) {
        self.inner.reset_efb();
    }

    fn take_draw_data(&mut self) -> Box<DrawData> {
        self.inner.take_draw_data()
    }
}

pub struct Playback<const SYSTEM: SystemId> {
    pub gx: GraphicsProcessor,
    pub mmio: Mmio<SYSTEM>,
}

impl<const SYSTEM: SystemId> Playback<SYSTEM> {
    pub fn new() -> Self {
        Playback {
            gx: GraphicsProcessor::new(),
            mmio: Mmio::new(),
        }
    }

    pub fn load_state(&mut self, file: &dff::DffFile, sink: &mut PlayerSink) {
        self.gx.load_tmem(&file.tex_mem);

        let mut stream: Vec<u8> = Vec::with_capacity(32 * 1024);

        for (i, val) in file.cp_mem.iter().enumerate().take(CP_REG_SIZE) {
            stream.push(0x08);
            stream.push(i as u8);
            stream.extend_from_slice(&val.to_be_bytes());
        }

        let xf_all: Vec<u32> = file.xf_mem.iter().chain(file.xf_regs.iter()).copied().collect();
        for start in (0..xf_all.len()).step_by(16) {
            let n = 16.min(xf_all.len() - start);

            stream.push(0x10);
            stream.extend_from_slice(&((n - 1) as u16).to_be_bytes());
            stream.extend_from_slice(&(start as u16).to_be_bytes());

            for val in &xf_all[start..start + n] {
                stream.extend_from_slice(&val.to_be_bytes());
            }
        }

        for (i, val) in file.bp_mem.iter().enumerate().take(BP_REG_SIZE) {
            if BP_RESTORE_SKIP.contains(&i) {
                continue;
            }

            stream.push(0x61);
            stream.push(i as u8);

            let v = val & 0x00ff_ffff;
            stream.extend_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, v as u8]);
        }

        self.feed(&stream, sink);
    }

    pub fn feed(&mut self, bytes: &[u8], sink: &mut PlayerSink) {
        self.gx.fifo.extend_from_slice(bytes);
        self.gx.drain_fifo(&mut self.mmio, sink);
    }

    pub fn play_frame(&mut self, frame: &dff::Frame, sink: &mut PlayerSink) -> bool {
        let fifo_data = &frame.fifo_data;
        let mut pos = 0usize;

        for update in &frame.memory_updates {
            let p = (update.fifo_position as usize).min(fifo_data.len());
            if p > pos {
                self.feed(&fifo_data[pos..p], sink);
                pos = p;
            }

            self.apply_update(update, sink);
        }

        if pos < fifo_data.len() {
            self.feed(&fifo_data[pos..], sink);
        }

        self.present(sink)
    }

    pub fn apply_update(&mut self, update: &dff::MemoryUpdate, sink: &mut PlayerSink) {
        {
            let mut ram = self.mmio.ram_view_mut();
            match ram.slice_mut(update.address as usize, update.data.len()) {
                Some(dst) => dst.copy_from_slice(&update.data),
                None => {
                    tracing::warn!(
                        addr = format!("{:#010X}", update.address),
                        len = update.data.len(),
                        "memory update outside RAM, skipping"
                    );
                    return;
                }
            }
        }

        if update.kind == dff::MemoryUpdateType::TextureMap && self.update_overlaps_bound_texture(update) {
            let view = self.mmio.ram_view();
            self.gx.refresh_bound_textures(sink, &view);
        }
    }

    fn update_overlaps_bound_texture(&self, update: &dff::MemoryUpdate) -> bool {
        let a = update.address as usize;
        let a_end = a + update.data.len();

        self.gx.cur_textures.iter().flatten().any(|desc| {
            let t = desc.ram_addr;
            let t_end = t + texture::raw_data_size(desc.width, desc.height, desc.format);

            a < t_end && t < a_end
        })
    }

    pub fn present(&mut self, sink: &mut PlayerSink) -> bool {
        let heights = std::mem::take(&mut sink.xfb_heights);

        if !self.gx.xfb_dirty {
            return false;
        }
        self.gx.xfb_dirty = false;

        let bytes_per_row = self
            .gx
            .xfb_regions
            .values()
            .max_by_key(|r| r.copy_seq)
            .map(|r| r.stride.max(2))
            .unwrap();
        let stride_px = bytes_per_row / 2;
        let min_base = self.gx.xfb_regions.keys().min().copied().unwrap();

        let mut parts: Vec<(u64, XfbPart)> = Vec::new();
        let mut frame_h = 0u32;

        for (&addr, region) in self.gx.xfb_regions.iter() {
            let delta_px = (addr - min_base) / 2;
            let offset_x = delta_px % stride_px;
            let offset_y = delta_px / stride_px;

            if offset_x != 0 {
                continue;
            }

            let dst_h = heights
                .iter()
                .rev()
                .find(|(id, _)| *id == addr)
                .map(|(_, h)| *h)
                .unwrap_or(0);

            frame_h = frame_h.max(offset_y + dst_h);

            parts.push((
                region.first_seq,
                XfbPart {
                    id: addr,
                    offset_x: 0,
                    offset_y,
                },
            ));
        }

        self.gx.xfb_regions.clear();

        parts.sort_by_key(|(first_seq, _)| *first_seq);
        let parts: Vec<XfbPart> = parts.into_iter().map(|(_, p)| p).collect();

        if parts.is_empty() || frame_h == 0 {
            return false;
        }

        sink.exec(GxAction::PresentXfb {
            width: stride_px,
            height: frame_h,
            parts,
        });

        true
    }
}
