pub mod disasm;
pub mod edits;
pub mod slice;
pub mod ui;

use crate::playback::{Playback, PlayerSink};
use edits::{EditModel, EffectiveFrame};
use gecko::host::{GxAction, RenderSink};
use gecko::system::SystemId;
use slice::{CmdKind, Command, CommandIndex, RamOverlay, SliceCtx};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum RunState {
    Paused,
    Running,
    RunTo { frame: usize, row: usize },
}

#[derive(Default)]
pub struct Breakpoints {
    pub on_draw: bool,
    pub on_cp: bool,
    pub on_xf: bool,
    pub on_bp: bool,
    pub on_calldl: bool,
    pub on_efb_copy: bool,
    pub cp_regs: HashSet<u8>,
    pub bp_regs: HashSet<u8>,
    pub xf_addrs: HashSet<u16>,
    pub rows: HashSet<(usize, usize)>,
}

impl Breakpoints {
    fn hits(&self, frame: usize, row: usize, cmd: &Command) -> bool {
        if self.rows.contains(&(frame, row)) {
            return true;
        }

        match &cmd.kind {
            CmdKind::Draw { .. } => self.on_draw,
            CmdKind::Cp { reg, .. } => self.on_cp || self.cp_regs.contains(reg),
            CmdKind::Bp { reg, .. } => {
                self.on_bp
                    || self.bp_regs.contains(reg)
                    || (self.on_efb_copy && *reg as usize == gecko::flipper::gx::constants::BP_PE_COPY_CMD)
            }
            CmdKind::Xf { addr, values } => {
                self.on_xf
                    || self
                        .xf_addrs
                        .iter()
                        .any(|a| (*addr..addr + values.len() as u16).contains(a))
            }
            CmdKind::CallDl { .. } => self.on_calldl,
            _ => false,
        }
    }
}

pub struct DebugSession<const SYSTEM: SystemId> {
    pub file: dff::DffFile,
    pub path: PathBuf,
    pub start: usize,
    pub end: usize,

    pub playback: Playback<SYSTEM>,
    pub sink: PlayerSink,

    pub edits: EditModel,
    pub breakpoints: Breakpoints,
    pub run_state: RunState,
    skip_bp_once: bool,

    pub frame_idx: usize,
    pub row: usize,
    next_update: usize,
    pub cur: EffectiveFrame,
    pub index: CommandIndex,
    frame_start_cp: Vec<u32>,

    pub finished: bool,
    pub presents: u64,
    pub index_version: u64,
}

impl<const SYSTEM: SystemId> DebugSession<SYSTEM> {
    pub fn new(file: dff::DffFile, path: PathBuf, start: usize, end: usize, mut sink: PlayerSink) -> Self {
        let mut playback = Playback::<SYSTEM>::new();
        playback.load_state(&file, &mut sink);

        let mut session = DebugSession {
            file,
            path,
            start,
            end,
            playback,
            sink,
            edits: EditModel::default(),
            breakpoints: Breakpoints::default(),
            run_state: RunState::Paused,
            skip_bp_once: false,
            frame_idx: start,
            row: 0,
            next_update: 0,
            cur: EffectiveFrame {
                fifo: Vec::new(),
                updates: Vec::new(),
                disabled: HashSet::new(),
                orig_of: Default::default(),
            },
            index: CommandIndex::default(),
            frame_start_cp: Vec::new(),
            finished: false,
            presents: 0,
            index_version: 0,
        };

        session.enter_frame(start);
        session
    }

    pub fn frame_count(&self) -> usize {
        self.file.frames.len()
    }

    pub fn byte_pos(&self) -> usize {
        match self.index.commands.get(self.row) {
            Some(cmd) => cmd.offset,
            None => self.cur.fifo.len(),
        }
    }

    pub fn at_frame_end(&self) -> bool {
        self.row >= self.index.commands.len()
    }

    pub fn applied_updates(&self) -> usize {
        self.next_update
    }

    fn enter_frame(&mut self, idx: usize) {
        self.frame_idx = idx;
        self.row = 0;
        self.next_update = 0;

        self.frame_start_cp = self.playback.gx.cp_regs.clone();
        self.cur = EffectiveFrame::build(&self.file.frames[idx], self.edits.frame(idx));
        self.rebuild_index();
    }

    fn rebuild_index(&mut self) {
        let overlay = RamOverlay {
            ram: self.playback.mmio.ram_view(),
            updates: &self.cur.updates,
        };
        let ctx = SliceCtx {
            overlay,
            disabled: &self.cur.disabled,
        };

        self.index = slice::slice_frame(&self.cur.fifo, &self.frame_start_cp, &ctx);
        self.index_version += 1;
    }

    fn apply_updates_through(&mut self, pos: usize) {
        let updates = std::mem::take(&mut self.cur.updates);

        while self.next_update < updates.len() && (updates[self.next_update].fifo_position as usize) < pos {
            self.playback.apply_update(&updates[self.next_update], &mut self.sink);
            self.next_update += 1;
        }

        self.cur.updates = updates;
    }

    fn exec_current(&mut self) -> bool {
        let cmd = &self.index.commands[self.row];
        let (off, end) = (cmd.offset, cmd.end());
        let draws = matches!(cmd.kind, CmdKind::Draw { .. } | CmdKind::CallDl { .. });

        self.apply_updates_through(end);

        if !self.cur.disabled.contains(&off) {
            let fifo = std::mem::take(&mut self.cur.fifo);
            self.playback.feed(&fifo[off..end], &mut self.sink);
            self.cur.fifo = fifo;
        }

        self.row += 1;
        draws
    }

    fn finish_frame(&mut self) {
        self.apply_updates_through(usize::MAX);

        if self.playback.present(&mut self.sink) {
            self.presents += 1;
        }

        if self.frame_idx >= self.end {
            self.finished = true;
            self.run_state = RunState::Paused;
        } else {
            self.enter_frame(self.frame_idx + 1);
        }
    }

    pub fn step_command(&mut self) -> bool {
        if self.finished {
            return false;
        }

        if self.at_frame_end() {
            self.finish_frame();
            false
        } else {
            self.exec_current()
        }
    }

    pub fn step_draw(&mut self) {
        while !self.finished {
            let crossed = self.at_frame_end();

            if self.step_command() || crossed {
                break;
            }
        }
    }

    pub fn step_frame(&mut self) {
        let f = self.frame_idx;

        while !self.finished && self.frame_idx == f {
            self.step_command();
        }
    }

    pub fn resume(&mut self, state: RunState) {
        if self.finished {
            return;
        }

        self.skip_bp_once = true;
        self.run_state = state;
    }

    pub fn pause(&mut self) {
        self.run_state = RunState::Paused;
    }

    pub fn run_tick(&mut self) {
        loop {
            if self.finished {
                self.run_state = RunState::Paused;
                return;
            }

            if let RunState::RunTo { frame, row } = self.run_state
                && self.frame_idx == frame
                && self.row == row
            {
                self.run_state = RunState::Paused;
                return;
            }

            if self.at_frame_end() {
                self.finish_frame();
                return;
            }

            let cmd = &self.index.commands[self.row];
            if !self.skip_bp_once && self.breakpoints.hits(self.frame_idx, self.row, cmd) {
                self.run_state = RunState::Paused;
                return;
            }

            self.skip_bp_once = false;
            self.exec_current();
        }
    }

    pub fn jump_to(&mut self, frame: usize, row: usize) {
        let frame = frame.clamp(self.start, self.end);
        let ahead = !self.finished && (frame > self.frame_idx || (frame == self.frame_idx && row >= self.row));

        if ahead {
            while !self.finished && (self.frame_idx, self.row) < (frame, row) {
                if self.frame_idx == frame && self.at_frame_end() {
                    break;
                }
                self.step_command();
            }
        } else {
            self.seek_to(frame, row);
        }

        self.run_state = RunState::Paused;
    }

    pub fn seek_to(&mut self, frame: usize, row: usize) {
        let frame = frame.clamp(self.start, self.end);

        self.playback = Playback::new();
        self.finished = false;

        self.sink.exec(GxAction::InvalidateCaches);
        self.sink.reset_efb();
        self.playback.load_state(&self.file, &mut self.sink);

        for f in self.start..frame {
            let presented = match self.edits.frame(f) {
                None => self.playback.play_frame(&self.file.frames[f], &mut self.sink),
                Some(fe) => {
                    let eff = EffectiveFrame::build(&self.file.frames[f], Some(fe));
                    let exec = eff.exec_fifo();
                    self.playback.play_frame_with(&exec, &eff.updates, &mut self.sink)
                }
            };

            if presented {
                self.presents += 1;
            }
        }

        self.enter_frame(frame);

        let row = row.min(self.index.commands.len());
        for _ in 0..row {
            if self.at_frame_end() {
                break;
            }

            self.exec_current();
        }

        self.run_state = RunState::Paused;
    }

    pub fn restart_frame(&mut self) {
        self.seek_to(self.frame_idx, 0);
    }

    pub fn restart(&mut self) {
        self.seek_to(self.start, 0);
    }

    pub fn toggle_disabled(&mut self, display_off: usize) {
        let Some(orig) = self.cur.pristine_offset(display_off) else {
            return;
        };

        let index = &self.index;
        let fe = self.edits.frame_mut(self.frame_idx, || self::boundaries_of(index));

        if !fe.disabled.remove(&orig) {
            fe.disabled.insert(orig);
        }

        self.after_edit(display_off);
    }

    pub fn replace_command(&mut self, display_off: usize, bytes: Vec<u8>) {
        let Some(orig) = self.cur.pristine_offset(display_off) else {
            return;
        };

        let index = &self.index;
        let fe = self.edits.frame_mut(self.frame_idx, || self::boundaries_of(index));
        fe.replacements.insert(orig, bytes);

        self.after_edit(display_off);
    }

    pub fn revert_command(&mut self, display_off: usize) {
        let Some(orig) = self.cur.pristine_offset(display_off) else {
            return;
        };

        if let Some(fe) = self.edits.frames.get_mut(&self.frame_idx) {
            fe.replacements.remove(&orig);
            fe.disabled.remove(&orig);
        }

        self.after_edit(display_off);
    }

    fn after_edit(&mut self, display_off: usize) {
        if display_off < self.byte_pos() {
            self.restart_frame();
        } else {
            self.cur = EffectiveFrame::build(&self.file.frames[self.frame_idx], self.edits.frame(self.frame_idx));
            self.rebuild_index();
        }
    }

    pub fn save_dff(&self) -> Result<PathBuf, dff::DffError> {
        let mut path = self.path.clone();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "dump".into());

        path.set_file_name(format!("{stem}.edited.dff"));
        self.edits.save_to_dff(&self.file, &path)?;

        Ok(path)
    }
}

fn boundaries_of(index: &CommandIndex) -> Vec<(usize, usize)> {
    index.commands.iter().map(|c| (c.offset, c.len)).collect()
}
