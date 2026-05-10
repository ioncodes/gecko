use clap::Parser;
use gecko::HostInput;
use gecko::wii::Wii;
use std::time::Instant;

#[derive(Parser)]
#[command(about = "Headless Wii benchmark (no renderer, no audio host)")]
struct Args {
    #[arg(long)]
    dvd: String,

    #[arg(long)]
    dsp: Option<String>,

    #[arg(long)]
    coef: Option<String>,

    #[arg(long, default_value_t = 60.0)]
    seconds: f64,

    #[arg(long, default_value_t = 60)]
    report_interval: u64,

    #[arg(long, default_value_t = 0.0)]
    warmup_seconds: f64,

    #[arg(long, default_value_t = false)]
    no_jit_cache: bool,
}

fn game_id_from_header(header: &image::dvd::Header) -> String {
    let mut buf = String::with_capacity(6);
    for &b in &header.game_code {
        buf.push(if b.is_ascii_graphic() { b as char } else { '_' });
    }
    for &b in &header.maker_code {
        buf.push(if b.is_ascii_graphic() { b as char } else { '_' });
    }
    buf
}

fn main() {
    let args = Args::parse();

    let dvd_data = std::fs::read(&args.dvd).expect("failed to read DVD");
    let dvd = image::load_dvd(dvd_data);
    assert!(dvd.header().is_wii(), "wii_bench requires a Wii disc");

    let game_id = game_id_from_header(dvd.header());

    let mut emulator = Wii::apploader_hle(dvd).build();

    if let Some(ref dsp_path) = args.dsp {
        let dsp_data = std::fs::read(dsp_path).expect("failed to read DSP IROM");
        emulator.dsp.load_irom(&dsp_data);
    }
    if let Some(ref coef_path) = args.coef {
        let coef_data = std::fs::read(coef_path).expect("failed to read DSP coef");
        emulator.dsp.load_coef(&coef_data);
    }

    if !args.no_jit_cache {
        let (ppc_c, ppc_s, dsp_c, dsp_s, vtx_c, vtx_s) = emulator.load_jit_cache(&game_id);
        if ppc_c + dsp_c + vtx_c + ppc_s + dsp_s + vtx_s > 0 {
            println!(
                "JIT cache loaded: ppc={}/{} dsp={}/{} vtx={}/{} (compiled/skipped)",
                ppc_c, ppc_s, dsp_c, dsp_s, vtx_c, vtx_s
            );
        } else {
            println!("JIT cache: empty (will be populated this run)");
        }
    }

    emulator.apply_host_input(&HostInput::wii_neutral());

    run_bench(&mut emulator, &args);

    if !args.no_jit_cache {
        match emulator.save_jit_cache(&game_id) {
            Ok((ppc, dsp, vtx)) => println!("JIT cache saved: ppc={} dsp={} vtx={}", ppc, dsp, vtx),
            Err(err) => eprintln!("save_jit_cache failed: {err}"),
        }
    }
}

fn run_bench(emulator: &mut Wii, args: &Args) {
    println!(
        "running {:.1}s (warmup {:.1}s, report every {} frames)",
        args.seconds, args.warmup_seconds, args.report_interval
    );
    println!();

    let start = Instant::now();
    let warmup_end = std::time::Duration::from_secs_f64(args.warmup_seconds);
    let total_end = std::time::Duration::from_secs_f64(args.warmup_seconds + args.seconds);

    let mut warmup_frames = 0u64;
    let mut measured_frames = 0u64;
    let mut interval_start = Instant::now();
    let mut interval_frames = 0u64;
    let mut interval_min = f64::MAX;
    let mut interval_max = 0.0_f64;
    let mut measure_start: Option<Instant> = None;

    loop {
        let frame_start = Instant::now();
        emulator.run_until_vsync();
        let frame_time = frame_start.elapsed().as_secs_f64();

        let elapsed = start.elapsed();
        let in_warmup = elapsed < warmup_end;
        if in_warmup {
            warmup_frames += 1;
        } else {
            if measure_start.is_none() {
                measure_start = Some(Instant::now());
                interval_start = Instant::now();
                interval_frames = 0;
                interval_min = f64::MAX;
                interval_max = 0.0;
                if warmup_frames > 0 {
                    println!("warmup done: {} frames", warmup_frames);
                }
            }
            measured_frames += 1;
            interval_frames += 1;
            if frame_time < interval_min {
                interval_min = frame_time;
            }
            if frame_time > interval_max {
                interval_max = frame_time;
            }

            if args.report_interval > 0 && interval_frames >= args.report_interval {
                let interval_elapsed = interval_start.elapsed().as_secs_f64();
                let avg_fps = interval_frames as f64 / interval_elapsed;
                let native_pct = (avg_fps / 60.0) * 100.0;
                let avg_ms = (interval_elapsed / interval_frames as f64) * 1000.0;
                let min_ms = interval_min * 1000.0;
                let max_ms = interval_max * 1000.0;
                println!(
                    "frame {:>7}  {:6.2} fps  ({:5.1}%)  avg {:5.2}ms  min {:5.2}ms  max {:5.2}ms",
                    measured_frames, avg_fps, native_pct, avg_ms, min_ms, max_ms
                );
                interval_start = Instant::now();
                interval_frames = 0;
                interval_min = f64::MAX;
                interval_max = 0.0;
            }
        }

        if elapsed >= total_end {
            break;
        }
    }

    let measure_dur = measure_start.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
    let avg_fps = if measure_dur > 0.0 {
        measured_frames as f64 / measure_dur
    } else {
        0.0
    };
    let native_pct = (avg_fps / 60.0) * 100.0;

    println!();
    println!("=== summary ===");
    println!("warmup frames:  {}", warmup_frames);
    println!("measured frames: {}", measured_frames);
    println!("measure window: {:.3}s", measure_dur);
    println!("avg fps:        {:.2}", avg_fps);
    println!("native:         {:.2}%", native_pct);
}
