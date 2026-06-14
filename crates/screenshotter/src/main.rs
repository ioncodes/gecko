use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const WORKER_COUNT: usize = 2;
const WORKER_BIN: &str = "screenshotter-worker";

const WORKER_MEM_MAX_DEFAULT: &str = "8G";
const WORKER_TIMEOUT_SECS: u64 = 300;

fn main() {
    let input_dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("Please provide a path to a folder of GameCube/Wii ISOs/RVZs"),
    );
    let gamelist = PathBuf::from(
        std::env::args()
            .nth(2)
            .expect("Please provide a path to a gamelist.txt file"),
    );

    let whitelist: std::collections::HashSet<String> = std::fs::read_to_string(&gamelist)
        .expect("Failed to read gamelist.txt")
        .lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect();

    let files: Vec<PathBuf> = std::fs::read_dir(&input_dir)
        .expect("Failed to read the provided path")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| matches!(path.extension().and_then(|e| e.to_str()), Some("iso" | "rvz" | "zip")))
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| whitelist.contains(n))
        })
        .collect();

    println!("Found {} files to process", files.len());

    let worker_exe = std::env::current_exe()
        .expect("could not resolve current_exe()")
        .parent()
        .expect("current_exe has no parent dir")
        .join(WORKER_BIN);

    if !worker_exe.is_file() {
        eprintln!(
            "Missing worker binary at {}. Build with `cargo build --release -p screenshotter --bins`.",
            worker_exe.display()
        );
        std::process::exit(1);
    }

    let mem_max = std::env::var("GECKO_WORKER_MEM_MAX").unwrap_or_else(|_| WORKER_MEM_MAX_DEFAULT.to_owned());
    let capped = cgroup_cap_available();
    if capped {
        println!(
            "Confining each worker to a {mem_max} memory-capped cgroup scope (override with GECKO_WORKER_MEM_MAX)."
        );
    } else {
        eprintln!("fuckyfucky");
    }

    let queue: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(files));
    let mut handles = Vec::with_capacity(WORKER_COUNT);
    for worker_id in 0..WORKER_COUNT {
        let queue = queue.clone();
        let worker_exe = worker_exe.clone();
        let mem_max = mem_max.clone();
        handles.push(
            std::thread::Builder::new()
                .name(format!("screenshotter-{worker_id}"))
                .spawn(move || {
                    loop {
                        let file = match queue.lock().unwrap().pop() {
                            Some(f) => f,
                            None => return,
                        };

                        match self::run_worker(
                            self::worker_command(&worker_exe, &file, capped, &mem_max),
                            capped,
                            &file,
                        ) {
                            Ok(status) if status.success() => {}
                            Ok(status) => {
                                eprintln!("Skipping {}: worker exited with {}", file.display(), status,);
                            }
                            Err(err) => {
                                eprintln!("Skipping {}: failed to spawn worker: {}", file.display(), err,);
                            }
                        }
                    }
                })
                .expect("failed to spawn pool thread"),
        );
    }

    for h in handles {
        let _ = h.join();
    }

    cleanup("screenshotdb");
}

fn worker_command(worker_exe: &Path, file: &Path, capped: bool, mem_max: &str) -> Command {
    if !capped {
        let mut cmd = Command::new(worker_exe);
        cmd.arg(file);
        return cmd;
    }

    let mut cmd = Command::new("systemd-run");
    cmd.args(["--user", "--scope", "--quiet", "--collect"])
        .arg("-p")
        .arg(format!("MemoryMax={mem_max}"))
        .arg("-p")
        .arg("MemorySwapMax=0")
        .arg("-p")
        .arg(format!("RuntimeMaxSec={WORKER_TIMEOUT_SECS}"))
        .arg("--")
        .arg(worker_exe)
        .arg(file);
    cmd
}

fn run_worker(mut cmd: Command, capped: bool, file: &Path) -> std::io::Result<std::process::ExitStatus> {
    if capped {
        return cmd.status();
    }

    let mut child = cmd.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(WORKER_TIMEOUT_SECS);

    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }

        if Instant::now() >= deadline {
            eprintln!(
                "Killing {}: worker exceeded {WORKER_TIMEOUT_SECS}s timeout",
                file.display()
            );
            let _ = child.kill();
            return child.wait();
        }

        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(target_os = "linux")]
fn cgroup_cap_available() -> bool {
    Command::new("systemd-run")
        .args(["--user", "--scope", "--quiet", "-p", "MemoryMax=64M", "--", "true"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn cgroup_cap_available() -> bool {
    false
}

fn hash_or_delete_unicolor(path: &Path) -> Option<u64> {
    let file = std::fs::File::open(path).ok()?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    reader.next_frame(&mut buf).ok()?;

    let samples = reader.info().color_type.samples();
    let bit_depth = reader.info().bit_depth as usize;
    let bytes_per_pixel = samples * bit_depth / 8;
    if bytes_per_pixel == 0 {
        return None;
    }

    let first = &buf[..bytes_per_pixel];
    if buf.chunks_exact(bytes_per_pixel).all(|px| px == first) {
        let _ = std::fs::remove_file(path);
        return None;
    }

    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    buf.hash(&mut hasher);
    Some(hasher.finish())
}

fn cleanup(root: &str) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };

    let pngs = entries
        .flatten()
        .flat_map(|game_dir| std::fs::read_dir(game_dir.path()).ok())
        .flat_map(|dir| dir.flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("png"));

    let mut by_hash: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for path in pngs {
        if let Some(h) = hash_or_delete_unicolor(&path) {
            by_hash.entry(h).or_default().push(path);
        }
    }

    for paths in by_hash.into_values().filter(|v| v.len() > 1) {
        for p in paths.into_iter().skip(1) {
            let _ = std::fs::remove_file(&p);
        }
    }
}
