use backend_wgpu::{GxRenderer, capture};
use gecko::HostInput;
use gecko::flipper::si::pad;
use gecko::flipper::vi::regs::RefreshRate;
use gecko::gamecube::GameCube;
use gecko::host::{GxAction, RenderSink};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const WORKER_COUNT: usize = 5;

const IPL: &[u8] = include_bytes!("../../../private/IPL.decoded.bin");
const DSP: &[u8] = include_bytes!("../../../private/dsp_rom.bin");

struct SyncSink {
    gx: Arc<Mutex<GxRenderer>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    scratch: Vec<gecko::host::DrawVertex>,
}

impl RenderSink for SyncSink {
    fn exec(&mut self, action: GxAction) {
        self.gx.lock().unwrap().process_action_with_external_scratch(
            &self.device,
            &self.queue,
            &action,
            &mut self.scratch,
        );
    }

    fn vertex_scratch(&mut self) -> &mut Vec<gecko::host::DrawVertex> {
        &mut self.scratch
    }
}

fn take_screenshot(device: &wgpu::Device, queue: &wgpu::Queue, gx: &GxRenderer, code: &str, frame: u32) {
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });

    let mut captured = capture::capture_texture(device, queue, &gx.xfb_texture).expect("capture_texture returned None");

    for px in captured.rgba.chunks_exact_mut(4) {
        px[3] = 255;
    }

    let path = format!("screenshotdb/{}/{}.png", code, frame);
    let file = std::fs::File::create(&path).expect("Failed to create PNG file");

    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), captured.width, captured.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);

    let mut writer = encoder.write_header().expect("Failed to write PNG header");
    writer
        .write_image_data(&captured.rgba)
        .expect("Failed to write PNG data");
}

fn main() {
    let input_dir = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("Please provide a path to a folder of GameCube ISOs/RVZs"),
    );
    let gamelist = std::path::PathBuf::from(
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

    let files = std::fs::read_dir(&input_dir)
        .expect("Failed to read the provided path")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| matches!(path.extension().and_then(|e| e.to_str()), Some("iso" | "rvz" | "zip")))
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| whitelist.contains(n))
        })
        .collect::<Vec<_>>();

    println!("Found {} files to process", files.len());

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no compatible wgpu adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .expect("failed to acquire wgpu device");

    let surface_format = wgpu::TextureFormat::Rgba8Unorm;

    let files = Arc::new(files);
    let next = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::with_capacity(WORKER_COUNT);
    for worker_id in 0..WORKER_COUNT {
        let files = files.clone();
        let next = next.clone();
        let device = device.clone();
        let queue = queue.clone();

        handles.push(
            std::thread::Builder::new()
                .name(format!("screenshotter-{worker_id}"))
                .spawn(move || {
                    loop {
                        let idx = next.fetch_add(1, Ordering::Relaxed);
                        if idx >= files.len() {
                            break;
                        }

                        let file = &files[idx];
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            run_one(&device, &queue, surface_format, file);
                        }));

                        if let Err(payload) = result {
                            let msg = payload
                                .downcast_ref::<String>()
                                .cloned()
                                .or_else(|| payload.downcast_ref::<&'static str>().map(|s| s.to_string()))
                                .unwrap_or_else(|| "<non-string panic>".to_string());
                            eprintln!("Skipping {}: emulator crashed: {}", file.display(), msg);
                        }
                    }
                })
                .expect("failed to spawn worker"),
        );
    }

    for h in handles {
        let _ = h.join();
    }

    cleanup("screenshotdb");
}

fn hash_or_delete_unicolor(path: &std::path::Path) -> Option<u64> {
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

    let mut by_hash: HashMap<u64, Vec<std::path::PathBuf>> = HashMap::new();
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

fn run_one(device: &wgpu::Device, queue: &wgpu::Queue, surface_format: wgpu::TextureFormat, file: &std::path::Path) {
    let buffer = std::fs::read(file).expect("Failed to read the file");
    let image = image::load_dvd(buffer);

    let name = String::from_utf8_lossy(&image.header().game_name);
    let name = name.trim_end_matches('\0').to_owned();
    let code = String::from_utf8_lossy(&image.header().game_code);
    let code = code.trim_end_matches('\0').to_owned();
    println!("Running: {} ({})", name, code);

    let out_dir = format!("screenshotdb/{}", code);
    std::fs::create_dir_all(&out_dir).expect("Failed to create screenshotdb directory");

    let gx = Arc::new(Mutex::new(GxRenderer::new(device, queue, surface_format)));

    let mut gamecube = GameCube::with_ipl(IPL, true);
    gamecube.dsp.load_irom(DSP);

    let mut input = HostInput::gc_connected();
    gamecube.apply_host_input(&input);

    gamecube.render_sink = Box::new(SyncSink {
        gx: gx.clone(),
        device: device.clone(),
        queue: queue.clone(),
        scratch: Vec::new(),
    });

    gamecube.insert_dvd(image);

    let framerate = match gamecube.vi.dcr.video_format().refresh_rate() {
        RefreshRate::Hz50 => 50,
        RefreshRate::Hz60 => 60,
    };

    // Preliminary for IPL skip
    for _ in 0..(framerate * 1) {
        gamecube.run_until_vsync();
    }

    let mut frame: u32 = framerate * 2;
    {
        let g = gx.lock().unwrap();
        take_screenshot(device, queue, &g, &code, frame);
    }

    for idx in 0..20 {
        if let HostInput::Gc(pad) = &mut input {
            pad.stick_y = pad::STICK_CENTER;
            pad.buttons = 0;

            if idx == 3 {
                pad.stick_y = 255;
            } else if idx > 3 && idx % 5 == 0 {
                pad.buttons = pad::A | pad::START;
            }
        }
        gamecube.apply_host_input(&input);

        for _ in 0..(framerate * 2) {
            gamecube.run_until_vsync();
            frame += 1;
        }

        let g = gx.lock().unwrap();
        take_screenshot(device, queue, &g, &code, frame);
    }
}
