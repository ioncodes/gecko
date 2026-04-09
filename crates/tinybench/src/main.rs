use clap::Parser;
use gecko::flipper::si::pad::PadStatus;
use gecko::gamecube::GameCube;
use gecko::scheduler::CYCLES_PER_VSYNC;
use std::time::Instant;

const CPU_CLOCK_HZ: f64 = 486_000_000.0;

#[derive(Parser)]
#[command(about = "Benchmark tool")]
struct Args {
    /// Path to IPL ROM
    #[arg(long)]
    ipl: String,

    /// Path to DSP IROM binary
    #[arg(long)]
    dsp: String,

    /// Path to a GameCube ISO
    #[arg(long)]
    iso: Option<String>,

    /// Number of frames to run (0 = unlimited)
    #[arg(long, default_value_t = 0)]
    frames: u64,

    /// Report stats every N frames (0 = summary only)
    #[arg(long, default_value_t = 60)]
    report_interval: u64,
}

fn main() {
    let args = Args::parse();

    let ipl_data = std::fs::read(&args.ipl).expect("failed to read IPL");
    let mut emulator = GameCube::with_ipl(&ipl_data);

    let dsp_data = std::fs::read(&args.dsp).expect("failed to read DSP IROM");
    emulator.dsp.load_irom(&dsp_data);

    if let Some(ref iso_path) = args.iso {
        let iso_data = std::fs::read(iso_path).expect("failed to read ISO");
        let dvd = image::dvd::Dvd::parse(iso_data);
        emulator.insert_dvd(dvd);
    }

    emulator.add_primary_controller(PadStatus {
        connected: true,
        ..PadStatus::default()
    });

    run_bench(&mut emulator, &args);
}

fn run_bench(emulator: &mut GameCube, args: &Args) {
    let mut frame_count: u64 = 0;
    let mut interval_frame_count: u64 = 0;

    let bench_start = Instant::now();
    let mut interval_start = Instant::now();

    let mut interval_min = f64::MAX;
    let mut interval_max = 0.0_f64;

    if args.frames > 0 {
        println!("Running {} frames...", args.frames);
    } else {
        println!("Running indefinitely (Ctrl-C to stop)");
    }
    println!();

    loop {
        let frame_start = Instant::now();

        emulator.run_until_vsync();
        emulator.gx.draw_commands.recycle();

        let frame_time = frame_start.elapsed().as_secs_f64();
        frame_count += 1;
        interval_frame_count += 1;

        if frame_time < interval_min {
            interval_min = frame_time;
        }
        if frame_time > interval_max {
            interval_max = frame_time;
        }

        if args.report_interval > 0 && interval_frame_count >= args.report_interval {
            let interval_elapsed = interval_start.elapsed().as_secs_f64();
            let avg_fps = interval_frame_count as f64 / interval_elapsed;
            let native_pct = (avg_fps / 60.0) * 100.0;
            let avg_ms = (interval_elapsed / interval_frame_count as f64) * 1000.0;
            let min_ms = interval_min * 1000.0;
            let max_ms = interval_max * 1000.0;

            println!(
                "frame {:>8} | {:6.1} fps ({:5.1}%) | {:.2}ms avg, {:.2}ms min, {:.2}ms max",
                frame_count, avg_fps, native_pct, avg_ms, min_ms, max_ms
            );

            interval_start = Instant::now();
            interval_frame_count = 0;
            interval_min = f64::MAX;
            interval_max = 0.0;
        }

        if args.frames > 0 && frame_count >= args.frames {
            break;
        }
    }

    let total_elapsed = bench_start.elapsed().as_secs_f64();
    let total_fps = frame_count as f64 / total_elapsed;
    let emulated_cycles = frame_count * CYCLES_PER_VSYNC;
    let emulated_seconds = emulated_cycles as f64 / CPU_CLOCK_HZ;
    let speed_pct = (emulated_seconds / total_elapsed) * 100.0;

    println!();
    println!("=== Summary ===");
    println!("Frames:        {}", frame_count);
    println!("Wall time:     {:.3}s", total_elapsed);
    println!("Emulated time: {:.3}s", emulated_seconds);
    println!("Average FPS:   {:.1}", total_fps);
    println!("Speed:         {:.1}% of native", speed_pct);
    println!("Cycles:        {}", emulated_cycles);
}
