use gecko::flipper::gx::constants::NOP_CMD;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

#[derive(Clone, Default)]
pub struct FrameEdits {
    pub boundaries: Vec<(usize, usize)>,
    pub disabled: HashSet<usize>,
    pub replacements: BTreeMap<usize, Vec<u8>>,
}

impl FrameEdits {
    pub fn is_empty(&self) -> bool {
        self.disabled.is_empty() && self.replacements.is_empty()
    }
}

#[derive(Default)]
pub struct EditModel {
    pub frames: HashMap<usize, FrameEdits>,
}

impl EditModel {
    pub fn frame(&self, idx: usize) -> Option<&FrameEdits> {
        self.frames.get(&idx).filter(|fe| !fe.is_empty())
    }

    pub fn frame_mut(&mut self, idx: usize, boundaries: impl FnOnce() -> Vec<(usize, usize)>) -> &mut FrameEdits {
        let fe = self.frames.entry(idx).or_default();

        if fe.boundaries.is_empty() {
            fe.boundaries = boundaries();
        }

        fe
    }

    pub fn any(&self) -> bool {
        self.frames.values().any(|fe| !fe.is_empty())
    }

    pub fn save_to_dff(&self, file: &dff::DffFile, path: &Path) -> Result<(), dff::DffError> {
        let mut out = file.clone();

        for (idx, frame) in out.frames.iter_mut().enumerate() {
            let Some(fe) = self.frame(idx) else { continue };

            let eff = EffectiveFrame::build(frame, Some(fe));
            frame.fifo_data = eff.exec_fifo().into_owned();
            frame.memory_updates = eff.updates;
        }

        out.save(path)
    }
}

pub struct EffectiveFrame {
    pub fifo: Vec<u8>,
    pub updates: Vec<dff::MemoryUpdate>,
    pub disabled: HashSet<usize>,
    pub orig_of: BTreeMap<usize, (usize, usize)>,
}

impl EffectiveFrame {
    pub fn build(frame: &dff::Frame, fe: Option<&FrameEdits>) -> Self {
        let Some(fe) = fe.filter(|fe| !fe.is_empty()) else {
            return EffectiveFrame {
                fifo: frame.fifo_data.clone(),
                updates: frame.memory_updates.clone(),
                disabled: HashSet::new(),
                orig_of: BTreeMap::new(),
            };
        };

        let orig = &frame.fifo_data;
        let mut fifo = Vec::with_capacity(orig.len());
        let mut disabled = HashSet::new();
        let mut orig_of = BTreeMap::new();
        let mut offset_map: Vec<(usize, usize)> = Vec::with_capacity(fe.boundaries.len() + 1);

        let mut covered = 0usize;
        for &(off, len) in &fe.boundaries {
            if off > covered {
                fifo.extend_from_slice(&orig[covered..off]);
            }

            let display_off = fifo.len();
            offset_map.push((off, display_off));

            match fe.replacements.get(&off) {
                Some(bytes) => fifo.extend_from_slice(bytes),
                None => fifo.extend_from_slice(&orig[off..off + len]),
            }

            orig_of.insert(display_off, (off, fifo.len() - display_off));
            if fe.disabled.contains(&off) {
                disabled.insert(display_off);
            }

            covered = off + len;
        }

        if covered < orig.len() {
            fifo.extend_from_slice(&orig[covered..]);
        }

        offset_map.push((covered, fifo.len().min(usize::MAX)));

        let remap = |p: usize| -> usize {
            match offset_map.binary_search_by_key(&p, |&(o, _)| o) {
                Ok(i) => offset_map[i].1,
                Err(0) => p,
                Err(i) => {
                    let (orig_start, disp_start) = offset_map[i - 1];
                    let span_end = offset_map.get(i).map_or(fifo.len(), |&(_, d)| d);
                    (disp_start + (p - orig_start)).min(span_end)
                }
            }
        };

        let mut updates = frame.memory_updates.clone();

        for update in &mut updates {
            update.fifo_position = remap(update.fifo_position as usize) as u32;
        }

        updates.sort_by_key(|u| u.fifo_position);

        EffectiveFrame {
            fifo,
            updates,
            disabled,
            orig_of,
        }
    }

    pub fn exec_fifo(&self) -> std::borrow::Cow<'_, [u8]> {
        if self.disabled.is_empty() {
            return std::borrow::Cow::Borrowed(&self.fifo);
        }

        let mut out = self.fifo.clone();

        for &off in &self.disabled {
            if let Some(&(_, len)) = self.orig_of.get(&off) {
                out[off..off + len].fill(NOP_CMD);
            }
        }

        std::borrow::Cow::Owned(out)
    }

    pub fn pristine_offset(&self, display_off: usize) -> Option<usize> {
        if self.orig_of.is_empty() {
            return Some(display_off);
        }

        self.orig_of.get(&display_off).map(|&(o, _)| o)
    }
}
