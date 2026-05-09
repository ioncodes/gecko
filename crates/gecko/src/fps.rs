use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::flipper::vi::regs::RefreshRate;
use crate::scheduler::cpu_clock;
use crate::system::{System, SystemId};

pub type FpsShared = Arc<AtomicU64>;

pub struct FpsCounter {
    pub vsync_count: u32,
    pub last_tick: Instant,
    pub run_start: Instant,
    pub shared: FpsShared,
    pub log_path: Option<PathBuf>,
}

impl FpsCounter {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            vsync_count: 0,
            last_tick: now,
            run_start: now,
            shared: Arc::new(AtomicU64::new(0)),
            log_path: None,
        }
    }

    pub fn shared(&self) -> FpsShared {
        Arc::clone(&self.shared)
    }
}

impl Default for FpsCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[inline(always)]
fn pack(fps: f32, native_pct: f32) -> u64 {
    ((fps.to_bits() as u64) << 32) | native_pct.to_bits() as u64
}

#[inline(always)]
pub fn read(shared: &FpsShared) -> (f32, f32) {
    let packed = shared.load(Ordering::Relaxed);
    let fps = f32::from_bits((packed >> 32) as u32);
    let native_pct = f32::from_bits(packed as u32);
    (fps, native_pct)
}

pub fn fps_handler<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    let now = Instant::now();
    let elapsed = now.duration_since(sys.fps_counter.last_tick).as_secs_f64();
    let vsyncs = sys.fps_counter.vsync_count;
    let fps = if elapsed > 0.0 { vsyncs as f64 / elapsed } else { 0.0 };

    let native_hz = match sys.vi.dcr.video_format().refresh_rate() {
        RefreshRate::Hz60 => 60.0,
        RefreshRate::Hz50 => 50.0,
    };
    let native_pct = if native_hz > 0.0 {
        (fps / native_hz) * 100.0
    } else {
        0.0
    };

    tracing::info!(
        vsyncs,
        elapsed_s = elapsed,
        fps = format!("{fps:.2}"),
        pct = format!("{native_pct:.1}"),
        "emu fps"
    );

    if let Some(path) = sys.fps_counter.log_path.clone() {
        use std::io::Write;

        let t = now.duration_since(sys.fps_counter.run_start).as_secs_f64();
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(f, "{t:.3},{fps:.2},{native_pct:.2},{vsyncs}");
        }
    }

    sys.fps_counter
        .shared
        .store(self::pack(fps as f32, native_pct as f32), Ordering::Relaxed);

    sys.fps_counter.vsync_count = 0;
    sys.fps_counter.last_tick = now;
    sys.scheduler
        .schedule_in(cpu_clock(SYSTEM), self::fps_handler::<SYSTEM>);
}
