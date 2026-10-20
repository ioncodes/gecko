use clap::Parser;
use gecko::system::{GC, System, SystemId, WII};
use gecko::{GameCube, HostInput, Wii};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser)]
#[command(about = "JIT bench shit")]
struct Args {
    #[arg(long)]
    dvd: PathBuf,
    #[arg(long)]
    data_dir: PathBuf,
    #[arg(long)]
    dsp: Option<PathBuf>,
    #[arg(long)]
    coef: Option<PathBuf>,
    #[arg(long)]
    load_state: Option<PathBuf>,
    #[arg(long, default_value_t = 600)]
    warmup_frames: u64,
    #[arg(long, default_value_t = 1200)]
    frames: u64,
}

fn main() {
    let args = Args::parse();
    assert!(args.frames > 0);

    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || run(args))
        .unwrap()
        .join()
        .unwrap();
}

fn run(args: Args) {
    std::fs::create_dir_all(&args.data_dir).unwrap();
    gecko::paths::set_base(&args.data_dir);

    let dvd = image::load_dvd(std::fs::read(&args.dvd).expect("read disc"));
    let game_id = dvd.header().game_id();
    println!("game_id={game_id}");
    gecko::profile::frame_timings::take();

    let start = Instant::now();
    if dvd.header().is_wii() {
        let emu = Wii::apploader_hle(dvd).build();
        bench::<WII>(emu, &args, start);
    } else {
        let emu = GameCube::with_ipl_hle(dvd);
        bench::<GC>(emu, &args, start);
    }
}

fn bench<const SYSTEM: SystemId>(mut emu: System<SYSTEM>, args: &Args, setup_start: Instant) {
    if let Some(path) = &args.dsp {
        emu.dsp.load_irom(&std::fs::read(path).expect("read DSP ROM"));
    }
    if let Some(path) = &args.coef {
        emu.dsp.load_coef(&std::fs::read(path).expect("read DSP coefficients"));
    }
    if let Some(path) = &args.load_state {
        emu.load_state_from_file(path).expect("load savestate");
    }
    emu.apply_host_input(&if SYSTEM == WII {
        HostInput::wii_neutral()
    } else {
        HostInput::gc_connected()
    });
    report("setup", 0, setup_start.elapsed().as_secs_f64());

    for (phase, frames) in [("warmup", args.warmup_frames), ("measured", args.frames)] {
        let start = Instant::now();
        for _ in 0..frames {
            emu.run_until_vsync();
        }
        report(phase, frames, start.elapsed().as_secs_f64());
    }

    let mut hash = std::hash::DefaultHasher::new();
    emu.mmio.ram.hash(&mut hash);
    emu.mmio.mem2.hash(&mut hash);
    emu.mmio.lcache.hash(&mut hash);
    emu.gekko.gprs.hash(&mut hash);
    emu.gekko.pc.hash(&mut hash);
    emu.gekko.cr.raw().hash(&mut hash);
    emu.gekko.spr.xer.raw().hash(&mut hash);
    emu.gekko.spr.lr.hash(&mut hash);
    emu.gekko.spr.ctr.hash(&mut hash);
    emu.gekko.fpscr.raw().hash(&mut hash);
    emu.scheduler.cycles.hash(&mut hash);

    for i in 0..32 {
        emu.gekko.read_fpr(i).to_bits().hash(&mut hash);
        emu.gekko.read_ps1(i).to_bits().hash(&mut hash);
    }

    println!("guest_hash={:016x} pc={:08x}", hash.finish(), emu.gekko.pc);
    println!("vertex_stats={:?}", emu.gx.jit_vtx.stats);
}

fn report(phase: &str, frames: u64, seconds: f64) {
    println!(
        "phase={phase} frames={frames} seconds={seconds:.9} fps={:.6}",
        frames as f64 / seconds
    );

    let timings = gecko::profile::frame_timings::take();
    for (name, (count, ns)) in ["ppc_compile", "dsp_compile", "vertex_compile", "dsp_run"]
        .into_iter()
        .zip(timings)
    {
        println!("phase={phase} metric={name} count={count} ns={ns}");
    }
}
