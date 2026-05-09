use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct HeatmapConfig {
    pub enabled: bool,
    pub interval_frames: u32,
    pub out_dir: PathBuf,
    pub top_k: usize,
}

impl Default for HeatmapConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_frames: 60,
            out_dir: PathBuf::from("./profile-dumps"),
            top_k: 64,
        }
    }
}

impl HeatmapConfig {
    pub fn ppc_csv_path(&self) -> PathBuf {
        self.out_dir.join("ppc-heatmap.csv")
    }

    pub fn dsp_csv_path(&self) -> PathBuf {
        self.out_dir.join("dsp-heatmap.csv")
    }
}

pub fn ensure_dir(path: &Path) -> std::io::Result<()> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(path)
}

#[cfg(any(feature = "profile", feature = "jit-stats"))]
pub fn resolve_symbol(addr: usize) -> Option<String> {
    let mut name: Option<String> = None;
    backtrace::resolve(addr as *mut std::ffi::c_void, |sym| {
        if name.is_none() {
            if let Some(n) = sym.name() {
                name = Some(format!("{:#}", n));
            }
        }
    });
    name
}

pub fn write_file_atomic(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }

    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        write(&mut file)?;
        file.sync_data().ok();
    }
    std::fs::rename(&tmp, path)
}

#[cfg(feature = "profile")]
#[derive(Clone, Debug)]
pub struct PprofConfig {
    pub hz: u32,
    pub secs: u32,
    pub out: PathBuf,
    pub delay_vsyncs: u32,
}

#[cfg(all(feature = "profile", windows))]
mod win_sampler {
    use super::*;
    use rustc_hash::FxHashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE};
    use windows_sys::Win32::System::Diagnostics::Debug::{CONTEXT, CONTEXT_FULL_AMD64, GetThreadContext};
    use windows_sys::Win32::System::Threading::{
        GetCurrentThreadId, OpenThread, ResumeThread, SuspendThread, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION,
        THREAD_SUSPEND_RESUME,
    };

    pub struct IpSampler {
        stop: Arc<AtomicBool>,
        joiner: Option<thread::JoinHandle<()>>,
        counts: Arc<Mutex<FxHashMap<u64, u64>>>,
        deadline: Instant,
        out: PathBuf,
        hz: u32,
        sample_total: Arc<std::sync::atomic::AtomicU64>,
    }

    #[repr(C, align(16))]
    struct AlignedContext(CONTEXT);

    impl IpSampler {
        pub fn start_for_current_thread(hz: u32, secs: u32, out: PathBuf) -> Result<Self, std::io::Error> {
            let tid = unsafe { GetCurrentThreadId() };
            let target: HANDLE = unsafe {
                OpenThread(
                    THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION,
                    FALSE,
                    tid,
                )
            };

            if target.is_null() {
                return Err(std::io::Error::last_os_error());
            }

            let stop = Arc::new(AtomicBool::new(false));
            let counts: Arc<Mutex<FxHashMap<u64, u64>>> = Arc::new(Mutex::new(FxHashMap::default()));
            let sample_total = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let deadline = Instant::now() + Duration::from_secs(secs as u64);
            let interval = Duration::from_nanos((1_000_000_000u64 / hz.max(1) as u64).max(1));

            let stop_c = stop.clone();
            let counts_c = counts.clone();
            let total_c = sample_total.clone();
            let target_addr = target as usize;

            let joiner = thread::Builder::new()
                .name("pprof-sampler".into())
                .spawn(move || {
                    let target = target_addr as HANDLE;
                    let mut ctx_box: Box<AlignedContext> = Box::new(unsafe { std::mem::zeroed() });
                    let mut local: Vec<(u64, u64)> = Vec::new();

                    while !stop_c.load(Ordering::Relaxed) {
                        let next = Instant::now() + interval;

                        ctx_box.0 = unsafe { std::mem::zeroed() };
                        ctx_box.0.ContextFlags = CONTEXT_FULL_AMD64;

                        let suspended = unsafe { SuspendThread(target) };
                        if suspended == u32::MAX {
                            break;
                        }

                        let ok = unsafe { GetThreadContext(target, &mut ctx_box.0) };
                        let rip = if ok != 0 { ctx_box.0.Rip } else { 0 };
                        unsafe { ResumeThread(target) };

                        if rip != 0 {
                            local.push((rip, 1));
                            total_c.fetch_add(1, Ordering::Relaxed);
                        }

                        if local.len() >= 1024 {
                            let mut counts = counts_c.lock().unwrap();
                            for (ip, n) in local.drain(..) {
                                *counts.entry(ip).or_insert(0) += n;
                            }
                        }

                        let now = Instant::now();
                        if next > now {
                            thread::sleep(next - now);
                        }
                    }

                    if !local.is_empty() {
                        let mut counts = counts_c.lock().unwrap();
                        for (ip, n) in local.drain(..) {
                            *counts.entry(ip).or_insert(0) += n;
                        }
                    }

                    unsafe { CloseHandle(target) };
                })
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

            Ok(Self {
                stop,
                joiner: Some(joiner),
                counts,
                deadline,
                out,
                hz,
                sample_total,
            })
        }

        pub fn expired(&self) -> bool {
            Instant::now() >= self.deadline
        }

        pub fn finish(mut self) -> std::io::Result<PathBuf> {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(j) = self.joiner.take() {
                let _ = j.join();
            }

            if let Some(parent) = self.out.parent() {
                ensure_dir(parent)?;
            }

            let counts: Vec<(u64, u64)> = {
                let c = self.counts.lock().unwrap();
                c.iter().map(|(&k, &v)| (k, v)).collect()
            };

            let mut resolved: FxHashMap<String, u64> = FxHashMap::default();
            let mut unresolved: u64 = 0;
            for (ip, n) in &counts {
                match super::resolve_symbol(*ip as usize) {
                    Some(n_str) => *resolved.entry(n_str).or_insert(0) += n,
                    None => unresolved += n,
                }
            }
            if unresolved > 0 {
                resolved.insert("<unresolved (likely JIT'd code)>".to_string(), unresolved);
            }

            let mut entries: Vec<(String, u64)> = resolved.into_iter().collect();
            entries.sort_by(|a, b| b.1.cmp(&a.1));

            let total: u64 = entries.iter().map(|(_, n)| *n).sum();
            let total_samples = self.sample_total.load(Ordering::Relaxed);

            use std::io::Write;
            let mut file = std::fs::File::create(&self.out)?;
            writeln!(file, "# samples={}, hz={}", total_samples, self.hz)?;
            writeln!(file, "rank,samples,pct,symbol")?;

            for (rank, (sym, n)) in entries.iter().enumerate() {
                let pct = (*n as f64) * 100.0 / (total.max(1) as f64);
                writeln!(file, "{},{},{:.2},{}", rank, n, pct, escape_csv(sym))?;
            }

            file.flush()?;
            Ok(self.out)
        }
    }

    fn escape_csv(s: &str) -> String {
        if s.contains(',') || s.contains('"') || s.contains('\n') {
            let mut out = String::with_capacity(s.len() + 2);
            out.push('"');

            for c in s.chars() {
                if c == '"' {
                    out.push('"');
                }
                out.push(c);
            }

            out.push('"');
            out
        } else {
            s.to_string()
        }
    }
}

#[cfg(all(feature = "profile", windows))]
pub use win_sampler::IpSampler;

#[cfg(all(feature = "profile", not(windows)))]
mod stub_sampler {
    use super::*;

    pub struct IpSampler;

    impl IpSampler {
        pub fn start_for_current_thread(_hz: u32, _secs: u32, _out: PathBuf) -> Result<Self, std::io::Error> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "in-process sampler is currently Windows-only",
            ))
        }

        pub fn expired(&self) -> bool {
            true
        }

        pub fn finish(self) -> std::io::Result<PathBuf> {
            Ok(PathBuf::new())
        }
    }
}

#[cfg(all(feature = "profile", not(windows)))]
pub use stub_sampler::IpSampler;
