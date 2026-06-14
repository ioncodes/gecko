use rustc_hash::FxHashMap;
use std::path::PathBuf;

use super::VtxKey;
use crate::host::DrawVertex;

const ULP_EXACT: u32 = 0;
const ULP_TRANSFORMED: u32 = 4;

#[derive(Clone, Copy, Debug)]
pub struct Mismatch {
    pub vertex_index: u32,
    pub field: Field,
    pub component: u8,
    pub jit_bits: u32,
    pub interp_bits: u32,
    pub ulp: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum Field {
    Position,
    Normal,
    Color0,
    Color1,
    PosView,
    Texcoord(u8),
}

impl Field {
    pub fn name(&self) -> String {
        match self {
            Field::Position => "position".into(),
            Field::Normal => "normal".into(),
            Field::Color0 => "color0".into(),
            Field::Color1 => "color1".into(),
            Field::PosView => "pos_view".into(),
            Field::Texcoord(i) => format!("texcoord{i}"),
        }
    }

    fn ulp_tolerance(&self) -> u32 {
        match self {
            Field::Position | Field::Color0 | Field::Color1 => ULP_EXACT,
            Field::Normal | Field::PosView | Field::Texcoord(_) => ULP_TRANSFORMED,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CompareCtx {
    pub key: VtxKey,
    pub draw_cmd: u8,
    pub vertex_count: u32,
}

pub fn compare_draw_vertices(jit: &[DrawVertex], interp: &[DrawVertex], _ctx: &CompareCtx) -> Vec<Mismatch> {
    const CAP: usize = 64;

    let mut out: Vec<Mismatch> = Vec::new();
    let n = jit.len().min(interp.len());

    for i in 0..n {
        let j = &jit[i];
        let p = &interp[i];

        cmp3(i, Field::Position, j.position, p.position, &mut out);
        cmp3(i, Field::Normal, j.normal, p.normal, &mut out);
        cmp3(i, Field::PosView, j.pos_view, p.pos_view, &mut out);
        cmp4(i, Field::Color0, j.color0, p.color0, &mut out);
        cmp4(i, Field::Color1, j.color1, p.color1, &mut out);

        for tc in 0..8 {
            cmp3(i, Field::Texcoord(tc as u8), j.texcoords[tc], p.texcoords[tc], &mut out);
        }

        if out.len() >= CAP {
            break;
        }
    }

    out
}

fn cmp3(idx: usize, f: Field, a: [f32; 3], b: [f32; 3], out: &mut Vec<Mismatch>) {
    for c in 0..3 {
        if let Some(m) = compare_one(idx, f, c as u8, a[c], b[c]) {
            out.push(m);
        }
    }
}

fn cmp4(idx: usize, f: Field, a: [f32; 4], b: [f32; 4], out: &mut Vec<Mismatch>) {
    for c in 0..4 {
        if let Some(m) = compare_one(idx, f, c as u8, a[c], b[c]) {
            out.push(m);
        }
    }
}

fn compare_one(idx: usize, f: Field, c: u8, jit: f32, interp: f32) -> Option<Mismatch> {
    let jb = jit.to_bits();
    let ib = interp.to_bits();

    if jb == ib {
        return None;
    }

    if jit.is_nan() && interp.is_nan() {
        return None;
    }

    let ulp = ulp_diff(jit, interp);
    let tol = f.ulp_tolerance();

    if ulp <= tol {
        return None;
    }

    Some(Mismatch {
        vertex_index: idx as u32,
        field: f,
        component: c,
        jit_bits: jb,
        interp_bits: ib,
        ulp,
    })
}

fn ulp_diff(a: f32, b: f32) -> u32 {
    let ai = a.to_bits() as i32;
    let bi = b.to_bits() as i32;

    fn map(x: i32) -> i64 {
        if x < 0 { (-(x as i64)) | (1i64 << 31) } else { x as i64 }
    }

    let d = (map(ai) - map(bi)).unsigned_abs();
    d.min(u32::MAX as u64) as u32
}

#[derive(Default, Clone, Debug)]
pub struct MismatchSummary {
    pub first_seen_draw: u64,
    pub total_mismatches: u64,
    pub fields_seen: rustc_hash::FxHashSet<Field>,
}

pub struct VertexJitValidator {
    pub summary: FxHashMap<(VtxKey, Field), MismatchSummary>,
    pub draw_seq: u64,
    pub interp_scratch: Vec<DrawVertex>,
}

impl VertexJitValidator {
    pub fn new() -> Self {
        Self {
            summary: FxHashMap::default(),
            draw_seq: 0,
            interp_scratch: Vec::with_capacity(256),
        }
    }

    pub fn record(&mut self, ctx: &CompareCtx, mismatches: &[Mismatch]) {
        self.draw_seq += 1;

        if mismatches.is_empty() {
            return;
        }

        for m in mismatches {
            let key = (ctx.key, m.field);
            let entry = self.summary.entry(key).or_insert_with(|| MismatchSummary {
                first_seen_draw: self.draw_seq,
                ..Default::default()
            });
            let was_new_field = entry.fields_seen.insert(m.field);

            entry.total_mismatches += 1;

            if was_new_field {
                tracing::error!(
                    cmd = ctx.draw_cmd,
                    vat = format!(
                        "{:08x}_{:08x}_{:08x}_{:08x}_{:08x}",
                        ctx.key.vcd_lo, ctx.key.vcd_hi, ctx.key.vat_a, ctx.key.vat_b, ctx.key.vat_c
                    ),
                    field = m.field.name(),
                    component = m.component,
                    jit_bits = format!("{:08x}", m.jit_bits),
                    interp_bits = format!("{:08x}", m.interp_bits),
                    ulp = m.ulp,
                    vertex = m.vertex_index,
                    "vtx JIT drift"
                );
            }
        }
    }

    pub fn dump_csv(&self, path: &PathBuf) -> std::io::Result<()> {
        use std::io::Write;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut f = std::fs::File::create(path)?;
        writeln!(
            f,
            "vcd_lo,vcd_hi,vat_a,vat_b,vat_c,field,first_seen_draw,total_mismatches"
        )?;

        for ((key, field), s) in &self.summary {
            writeln!(
                f,
                "{:08x},{:08x},{:08x},{:08x},{:08x},{},{},{}",
                key.vcd_lo,
                key.vcd_hi,
                key.vat_a,
                key.vat_b,
                key.vat_c,
                field.name(),
                s.first_seen_draw,
                s.total_mismatches,
            )?;
        }

        Ok(())
    }
}

impl Default for VertexJitValidator {
    fn default() -> Self {
        Self::new()
    }
}
