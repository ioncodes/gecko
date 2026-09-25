use super::GraphicsProcessor;
use super::constants::{BP_REG_SIZE, CP_REG_SIZE};
use crate::mmio::RamView;
use crate::mmio::constants::MEM2_BASE;
pub use dff::MemoryUpdateType;
use dff::{DffFile, Frame, MemoryUpdate};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecorderState {
    Waiting,
    Recording,
    Done,
}

pub struct FifoRecorder {
    state: RecorderState,
    frames_recorded: u32,
    file: DffFile,
    cur_frame: Frame,
    shadow_mem1: Vec<u8>,
    shadow_mem2: Vec<u8>,
    pending_efb_ranges: Vec<(u32, usize)>,
    seen_draw_textures: rustc_hash::FxHashSet<(u32, usize)>,
    xfb_copy_in_flight: bool,
    last_xfb_copy_end: Option<usize>,
    total_fifo_bytes: u64,
    total_update_bytes: u64,
}

impl FifoRecorder {
    pub fn new() -> Self {
        FifoRecorder {
            state: RecorderState::Waiting,
            frames_recorded: 0,
            file: DffFile::default(),
            cur_frame: Frame::default(),
            shadow_mem1: Vec::new(),
            shadow_mem2: Vec::new(),
            pending_efb_ranges: Vec::new(),
            seen_draw_textures: rustc_hash::FxHashSet::default(),
            xfb_copy_in_flight: false,
            last_xfb_copy_end: None,
            total_fifo_bytes: 0,
            total_update_bytes: 0,
        }
    }

    pub fn state(&self) -> RecorderState {
        self.state
    }

    pub fn frames_recorded(&self) -> u32 {
        self.frames_recorded
    }

    pub fn fifo_bytes(&self) -> u64 {
        self.total_fifo_bytes
    }

    pub fn update_bytes(&self) -> u64 {
        self.total_update_bytes
    }

    pub fn request_stop(&mut self) {
        self.state = RecorderState::Done;
    }

    pub fn into_file(self) -> DffFile {
        self.file
    }

    #[inline(always)]
    pub fn is_recording(&self) -> bool {
        self.state == RecorderState::Recording
    }

    #[inline]
    pub fn record_command(&mut self, bytes: &[u8]) {
        if self.is_recording() {
            self.cur_frame.fifo_data.extend_from_slice(bytes);
            self.total_fifo_bytes += bytes.len() as u64;

            if self.xfb_copy_in_flight {
                self.xfb_copy_in_flight = false;
                self.last_xfb_copy_end = Some(self.cur_frame.fifo_data.len());
            }
        }
    }

    pub fn note_xfb_copy(&mut self) {
        if self.is_recording() {
            self.xfb_copy_in_flight = true;
        }
    }

    pub fn use_memory(&mut self, ram: &RamView<'_>, addr: u32, len: usize, kind: MemoryUpdateType) {
        if !self.is_recording() || len == 0 {
            return;
        }

        let Some(cur) = ram.slice(addr as usize, len) else {
            return;
        };
        let Some(shadow) = self::shadow_range(&mut self.shadow_mem1, &mut self.shadow_mem2, ram, addr, len) else {
            return;
        };

        if shadow != cur {
            shadow.copy_from_slice(cur);

            self.total_update_bytes += len as u64;
            self.cur_frame.memory_updates.push(MemoryUpdate {
                fifo_position: self.cur_frame.fifo_data.len() as u32,
                address: addr,
                kind,
                data: cur.to_vec(),
            });
        }
    }

    pub fn use_draw_texture(&mut self, ram: &RamView<'_>, addr: u32, len: usize) {
        if !self.is_recording() || !self.seen_draw_textures.insert((addr, len)) {
            return;
        }
        self.use_memory(ram, addr, len, MemoryUpdateType::TextureMap);
    }

    pub fn note_efb_copy(&mut self, addr: u32, len: usize) {
        if self.is_recording() && len != 0 {
            self.pending_efb_ranges.push((addr, len));
        }
    }

    pub fn flush_efb_shadow(&mut self, ram: &RamView<'_>) {
        while let Some((addr, len)) = self.pending_efb_ranges.pop() {
            let Some(cur) = ram.slice(addr as usize, len) else {
                continue;
            };

            if let Some(shadow) = self::shadow_range(&mut self.shadow_mem1, &mut self.shadow_mem2, ram, addr, len) {
                shadow.copy_from_slice(cur);
            }
        }
    }

    pub fn on_frame_boundary(&mut self, gp: &GraphicsProcessor, fifo_start: u32, fifo_end: u32, is_wii: bool) {
        match self.state {
            RecorderState::Waiting => {
                self.snapshot_state(gp, is_wii);
                self.state = RecorderState::Recording;
            }
            RecorderState::Recording => {
                let Some(end) = self.last_xfb_copy_end.take() else {
                    return;
                };
                self.seen_draw_textures.clear();

                let mut frame = std::mem::take(&mut self.cur_frame);
                let tail_data = frame.fifo_data.split_off(end);
                let split = frame
                    .memory_updates
                    .partition_point(|u| (u.fifo_position as usize) < end);
                let tail_updates = frame
                    .memory_updates
                    .split_off(split)
                    .into_iter()
                    .map(|mut u| {
                        u.fifo_position -= end as u32;
                        u
                    })
                    .collect();
                self.cur_frame = Frame {
                    fifo_data: tail_data,
                    fifo_start: 0,
                    fifo_end: 0,
                    memory_updates: tail_updates,
                };

                frame.fifo_start = fifo_start;
                frame.fifo_end = fifo_end;

                self.file.frames.push(frame);
                self.frames_recorded += 1;
            }
            RecorderState::Done => {}
        }
    }

    fn snapshot_state(&mut self, gp: &GraphicsProcessor, is_wii: bool) {
        self.file.is_wii = is_wii;
        self.file.bp_mem[..BP_REG_SIZE].copy_from_slice(&gp.bp_regs[..BP_REG_SIZE]);
        self.file.cp_mem[..CP_REG_SIZE].copy_from_slice(&gp.cp_regs[..CP_REG_SIZE]);
        self.file.xf_mem.copy_from_slice(&gp.xf_mem[..dff::XF_MEM_SIZE]);
        let regs = &gp.xf_mem[dff::XF_MEM_SIZE..dff::XF_MEM_SIZE + dff::XF_REGS_SIZE];
        self.file.xf_regs.copy_from_slice(regs);

        for (i, entry) in gp.tmem.iter().enumerate() {
            let off = i * 2;
            if off + 2 > self.file.tex_mem.len() {
                break;
            }

            self.file.tex_mem[off..off + 2].copy_from_slice(&entry.to_be_bytes());
        }
    }
}

fn shadow_range<'a>(
    shadow_mem1: &'a mut Vec<u8>,
    shadow_mem2: &'a mut Vec<u8>,
    ram: &RamView<'_>,
    addr: u32,
    len: usize,
) -> Option<&'a mut [u8]> {
    let addr = addr as usize;

    if addr < ram.mem1.len() {
        if shadow_mem1.len() != ram.mem1.len() {
            shadow_mem1.resize(ram.mem1.len(), 0);
        }

        shadow_mem1.get_mut(addr..addr + len)
    } else if addr >= MEM2_BASE as usize {
        if shadow_mem2.len() != ram.mem2.len() {
            shadow_mem2.resize(ram.mem2.len(), 0);
        }

        let off = addr - MEM2_BASE as usize;
        shadow_mem2.get_mut(off..off + len)
    } else {
        None
    }
}
