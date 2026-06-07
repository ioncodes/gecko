use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::game::{CpuMode, ThemePreference};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub gcn_library: Option<PathBuf>,
    pub wii_library: Option<PathBuf>,
    pub cpu_mode: CpuMode,
    pub theme: ThemePreference,
    pub system_dir: Option<PathBuf>,
    pub dsp_rom: Option<PathBuf>,
    pub dsp_coef: Option<PathBuf>,
    pub ipl: Option<PathBuf>,
    #[serde(default = "self::default_skip_ipl")]
    pub skip_ipl: bool,
}

fn default_skip_ipl() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gcn_library: None,
            wii_library: None,
            cpu_mode: CpuMode::default(),
            theme: ThemePreference::default(),
            system_dir: None,
            dsp_rom: None,
            dsp_coef: None,
            ipl: None,
            skip_ipl: self::default_skip_ipl(),
        }
    }
}

pub const DSP_ROM_FILE: &str = "dsp_rom.bin";
pub const DSP_COEF_FILE: &str = "dsp_coef.bin";
pub const IPL_FILE: &str = "IPL.bin";

impl Config {
    pub fn system_dir_resolved(&self) -> PathBuf {
        self.system_dir.clone().unwrap_or_else(|| self::exe_relative("system"))
    }

    pub fn resolve_in_dir(override_path: &Option<PathBuf>, system_dir: &Path, name: &str) -> Option<PathBuf> {
        if let Some(p) = override_path {
            return Some(p.clone());
        }
        let candidate = system_dir.join(name);
        candidate.exists().then_some(candidate)
    }
}

pub fn exe_relative(rel: impl AsRef<Path>) -> PathBuf {
    let rel = rel.as_ref();
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join(rel)))
        .unwrap_or_else(|| rel.to_path_buf())
}

pub fn config_path() -> PathBuf {
    self::exe_relative("config.toml")
}

pub fn load(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(s) => match toml::from_str(&s) {
            Ok(cfg) => cfg,
            Err(err) => {
                tracing::warn!(?err, path = %path.display(), "failed to parse config; using defaults");
                Config::default()
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(err) => {
            tracing::warn!(?err, path = %path.display(), "failed to read config; using defaults");
            Config::default()
        }
    }
}

pub fn save(path: &Path, cfg: &Config) -> anyhow::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let body = toml::to_string_pretty(cfg)?;
    std::fs::write(path, body)?;
    Ok(())
}
