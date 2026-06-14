use std::time::Duration;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "hostinput=debug".into()),
        )
        .init();

    let service = hostinput::sdl::service();

    loop {
        std::thread::sleep(Duration::from_millis(500));

        let Some(p) = *service.shared.lock().unwrap() else {
            continue;
        };

        println!(
            "{:?} buttons={:019b} left=({:+.2},{:+.2}) right=({:+.2},{:+.2}) l2={:.2} r2={:.2} accel=({:+.2},{:+.2},{:+.2}) pointer={:?}",
            p.caps,
            p.state.buttons,
            p.state.left.0,
            p.state.left.1,
            p.state.right.0,
            p.state.right.1,
            p.state.l2,
            p.state.r2,
            p.state.accel[0],
            p.state.accel[1],
            p.state.accel[2],
            p.pointer,
        );
    }
}
