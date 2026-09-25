use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::game::{AspectMode, CpuMode, ThemePreference};
use crate::keybinds::KeyboardConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub setup_completed: bool,
    pub gcn_library: Option<PathBuf>,
    pub wii_library: Option<PathBuf>,
    pub cpu_mode: CpuMode,
    pub theme: ThemePreference,
    pub system_dir: Option<PathBuf>,
    pub dsp_rom: Option<PathBuf>,
    pub dsp_coef: Option<PathBuf>,
    pub ipl: Option<PathBuf>,
    pub ipl_hle: bool,
    #[serde(default = "self::default_skip_ipl")]
    pub skip_ipl: bool,
    #[serde(default = "self::default_upscale")]
    pub upscale: u32,
    pub aspect: AspectMode,
    #[serde(default = "self::default_memcard_enabled")]
    pub memcard_enabled: bool,
    #[serde(default = "self::default_sram_enabled")]
    pub sram_enabled: bool,
    pub input: hostinput::InputConfig,
    pub keyboard: KeyboardConfig,
    #[serde(default)]
    pub wii_presets: Option<WiiPresets>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WiiPresetId {
    #[default]
    Upright,
    Sideways,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WiiPreset {
    pub controller: hostinput::config::WiiConfig,
    pub keyboard: crate::keybinds::WiiKeysConfig,
}

impl WiiPreset {
    pub fn defaults(id: WiiPresetId) -> Self {
        let mut preset = Self::default();
        let sideways = id == WiiPresetId::Sideways;
        preset.controller.sideways = Some(sideways);
        preset.controller.nunchuk_attached = Some(!sideways);
        if sideways {
            preset.controller.left_stick_dpad = Some(true);
            preset.controller.one = Some("west".into());
            preset.controller.two = Some("south".into());
            preset.controller.a = Some("north".into());
            preset.keyboard.one = Some("z".into());
            preset.keyboard.two = Some("x".into());
        }
        preset
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WiiPresets {
    pub active: WiiPresetId,
    pub upright: WiiPreset,
    pub sideways: WiiPreset,
}

impl Default for WiiPresets {
    fn default() -> Self {
        Self {
            active: WiiPresetId::Upright,
            upright: WiiPreset::defaults(WiiPresetId::Upright),
            sideways: WiiPreset::defaults(WiiPresetId::Sideways),
        }
    }
}

impl WiiPresets {
    fn selected(&self) -> &WiiPreset {
        match self.active {
            WiiPresetId::Upright => &self.upright,
            WiiPresetId::Sideways => &self.sideways,
        }
    }

    fn selected_mut(&mut self) -> &mut WiiPreset {
        match self.active {
            WiiPresetId::Upright => &mut self.upright,
            WiiPresetId::Sideways => &mut self.sideways,
        }
    }
}

impl Config {
    pub fn active_wii_preset(&self) -> WiiPresetId {
        self.wii_presets.as_ref().map(|p| p.active).unwrap_or_default()
    }

    pub fn store_wii_preset(&mut self) {
        let presets = self.wii_presets.get_or_insert_with(WiiPresets::default);
        self.input.wii.sideways = Some(presets.active == WiiPresetId::Sideways);
        *presets.selected_mut() = WiiPreset {
            controller: self.input.wii.clone(),
            keyboard: self.keyboard.wii.clone(),
        };
    }

    fn restore_wii_preset(&mut self) {
        let presets = self.wii_presets.as_ref().unwrap();
        self.input.wii = presets.selected().controller.clone();
        self.input.wii.sideways = Some(presets.active == WiiPresetId::Sideways);
        self.keyboard.wii = presets.selected().keyboard.clone();
    }

    pub fn select_wii_preset(&mut self, id: WiiPresetId) {
        self.store_wii_preset();
        self.wii_presets.as_mut().unwrap().active = id;
        self.restore_wii_preset();
    }

    pub fn reset_wii_preset(&mut self) {
        let id = self.active_wii_preset();
        *self.wii_presets.as_mut().unwrap().selected_mut() = WiiPreset::defaults(id);
        self.restore_wii_preset();
    }

    fn initialize_wii_presets(&mut self) {
        if self.wii_presets.is_none() {
            let mut presets = WiiPresets::default();
            if self.input.wii.sideways.unwrap_or(false) {
                presets.active = WiiPresetId::Sideways;
            }
            *presets.selected_mut() = WiiPreset {
                controller: self.input.wii.clone(),
                keyboard: self.keyboard.wii.clone(),
            };
            self.wii_presets = Some(presets);
        }
        self.restore_wii_preset();
    }
}

fn default_skip_ipl() -> bool {
    true
}

fn default_upscale() -> u32 {
    1
}

fn default_memcard_enabled() -> bool {
    true
}

fn default_sram_enabled() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            setup_completed: false,
            gcn_library: None,
            wii_library: None,
            cpu_mode: CpuMode::default(),
            theme: ThemePreference::default(),
            system_dir: None,
            dsp_rom: None,
            dsp_coef: None,
            ipl: None,
            ipl_hle: false,
            skip_ipl: self::default_skip_ipl(),
            upscale: self::default_upscale(),
            aspect: AspectMode::default(),
            memcard_enabled: self::default_memcard_enabled(),
            sram_enabled: self::default_sram_enabled(),
            input: hostinput::InputConfig::default(),
            keyboard: KeyboardConfig::default(),
            wii_presets: Some(WiiPresets::default()),
        }
    }
}

/// Console-internal storage (SRAM, memory cards) lives in the data directory.
pub const SRAM_FILE: &str = "internal/sram.bin";
pub const MEMCARD_A_FILE: &str = "internal/memcard_a.raw";

pub const DSP_ROM_FILE: &str = "dsp_rom.bin";
pub const DSP_COEF_FILE: &str = "dsp_coef.bin";
pub const IPL_FILE: &str = "IPL.bin";

impl Config {
    pub fn system_dir_resolved(&self) -> PathBuf {
        self.system_dir.clone().unwrap_or_else(|| self::data_relative("system"))
    }

    pub fn resolve_in_dir(override_path: &Option<PathBuf>, system_dir: &Path, name: &str) -> Option<PathBuf> {
        if let Some(p) = override_path {
            return Some(p.clone());
        }
        let candidate = system_dir.join(name);
        candidate.exists().then_some(candidate)
    }
}

pub fn data_relative(rel: impl AsRef<Path>) -> PathBuf {
    gecko::paths::resolve(rel)
}

pub fn config_path() -> PathBuf {
    self::data_relative("config.toml")
}

pub fn load(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(s) => match toml::from_str::<Config>(&s) {
            Ok(mut cfg) => {
                cfg.initialize_wii_presets();
                cfg
            }
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
