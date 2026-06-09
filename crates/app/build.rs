use std::path::Path;
use std::process::Command;

fn main() {
    let hash = self::git_output(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    let sha = self::git_output(&["rev-parse", "HEAD"]).unwrap_or_default();
    let date = self::git_output(&[
        "show",
        "-s",
        "--date=format-local:%Y-%m-%dT%H:%M:%SZ",
        "--format=%cd",
        "HEAD",
    ])
    .unwrap_or_default();

    let branch = std::env::var("GECKO_CHANNEL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| self::git_output(&["rev-parse", "--abbrev-ref", "HEAD"]))
        .unwrap_or_default();
    let channel = if branch == "dev" || branch == "nightly" {
        "nightly"
    } else {
        ""
    };

    println!("cargo:rustc-env=GECKO_GIT_HASH={hash}");
    println!("cargo:rustc-env=GECKO_GIT_SHA={sha}");
    println!("cargo:rustc-env=GECKO_COMMIT_DATE={date}");
    println!("cargo:rustc-env=GECKO_CHANNEL={channel}");
    println!("cargo:rerun-if-env-changed=GECKO_CHANNEL");

    let git_dir = Path::new("../../.git");
    let head = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head.display());

    if let Ok(contents) = std::fs::read_to_string(&head)
        && let Some(reference) = contents.strip_prefix("ref:")
    {
        let ref_path = git_dir.join(reference.trim());
        println!("cargo:rerun-if-changed={}", ref_path.display());
    }
}

fn git_output(args: &[&str]) -> Option<String> {
    Command::new("git")
        .env("TZ", "UTC")
        .args(args)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}
