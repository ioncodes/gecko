mod app;
mod cache;
mod config;
mod game;
mod library;
mod player;
mod theme;
mod widgets;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::path::PathBuf;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug, Clone)]
#[command(version, about = "Gecko: A humble GameCube/Wii emulator")]
struct Args {
    #[arg(long)]
    gcn: Option<PathBuf>,
    #[arg(long)]
    wii: Option<PathBuf>,
}

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")))
        .init();

    let args = Args::parse();

    iced::daemon(
        move || app::App::boot(args.gcn.clone(), args.wii.clone()),
        app::App::update,
        app::App::view,
    )
    .title(app::App::title)
    .theme(app::App::theme)
    .subscription(app::App::subscription)
    .run()
}
