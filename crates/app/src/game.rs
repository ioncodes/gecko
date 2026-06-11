use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use iced::widget::image::Handle;
use image::Dvd;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BannerData {
    pub width: u32,
    pub height: u32,
    #[serde(with = "serde_bytes")]
    pub rgba: Vec<u8>,
}

impl From<image::banner::Banner> for BannerData {
    fn from(b: image::banner::Banner) -> Self {
        Self {
            width: b.width,
            height: b.height,
            rgba: b.rgba,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Region {
    NtscU,
    NtscJ,
    Pal,
    NtscK,
    NtscT,
    Unknown,
}

impl Region {
    pub fn from_game_code(code: [u8; 4]) -> Self {
        match code[3] {
            b'E' | b'N' | b'B' => Region::NtscU,
            b'J' => Region::NtscJ,
            b'P' | b'D' | b'F' | b'S' | b'I' | b'H' | b'U' | b'X' | b'Y' | b'Z' => Region::Pal,
            b'K' | b'Q' | b'T' => Region::NtscK,
            b'W' => Region::NtscT,
            _ => Region::Unknown,
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Region::NtscU => "NTSC-U",
            Region::NtscJ => "NTSC-J",
            Region::Pal => "PAL",
            Region::NtscK => "NTSC-K",
            Region::NtscT => "NTSC-T",
            Region::Unknown => "??",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    Gcn,
    Wii,
}

impl Platform {
    pub fn short(self) -> &'static str {
        match self {
            Platform::Gcn => "GCN",
            Platform::Wii => "Wii",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    Iso,
    Rvz,
    Zip,
    Dol,
}

impl Format {
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "iso" => Some(Format::Iso),
            "rvz" => Some(Format::Rvz),
            "zip" => Some(Format::Zip),
            _ => None,
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Format::Iso => "ISO",
            Format::Rvz => "RVZ",
            Format::Zip => "ZIP",
            Format::Dol => "DOL",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CpuMode {
    #[default]
    Jit,
    Interpreter,
}

impl CpuMode {
    pub fn label(self) -> &'static str {
        match self {
            CpuMode::Jit => "JIT",
            CpuMode::Interpreter => "Interpreter",
        }
    }
}

impl From<CpuMode> for gecko::ExecutionMode {
    fn from(mode: CpuMode) -> Self {
        match mode {
            CpuMode::Jit => gecko::ExecutionMode::Jit,
            CpuMode::Interpreter => gecko::ExecutionMode::Interpreter,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    pub path: PathBuf,
    pub file_name: String,
    pub game_id: String,
    pub maker_code: String,
    pub disc_id: u8,
    pub title: String,
    pub region: Region,
    pub platform: Platform,
    pub format: Format,
    pub banner: Option<Arc<BannerData>>,
    #[serde(skip)]
    pub title_lc: String,
    #[serde(skip)]
    pub file_name_lc: String,
    #[serde(skip)]
    pub game_id_lc: String,
    #[serde(skip, default)]
    pub banner_handle: Arc<OnceLock<Handle>>,
}

impl Game {
    pub fn from_dvd(path: &Path, dvd: &dyn Dvd, format: Format) -> Self {
        let hdr = dvd.header();
        let title = self::decode_cstring(&hdr.game_name);
        let game_id = String::from_utf8_lossy(&hdr.game_code).into_owned();
        let maker_code = String::from_utf8_lossy(&hdr.maker_code).into_owned();
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_owned();

        let region = Region::from_game_code(hdr.game_code);
        let platform = if hdr.is_wii() { Platform::Wii } else { Platform::Gcn };
        let banner = image::banner::extract(dvd).map(BannerData::from).map(Arc::new);

        let title_lc = title.to_lowercase();
        let file_name_lc = file_name.to_lowercase();
        let game_id_lc = game_id.to_lowercase();

        Game {
            path: path.to_owned(),
            file_name,
            game_id,
            maker_code,
            disc_id: hdr.disk_id,
            title,
            region,
            platform,
            format,
            banner,
            title_lc,
            file_name_lc,
            game_id_lc,
            banner_handle: Arc::new(OnceLock::new()),
        }
    }

    pub fn from_dol(path: &Path, data: &[u8], platform: Platform) -> Self {
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_owned();
        let title = path.file_stem().and_then(|s| s.to_str()).unwrap_or("DOL").to_owned();
        let game_id = gecko::jit::cache::dol_cache_id(data);

        let title_lc = title.to_lowercase();
        let file_name_lc = file_name.to_lowercase();
        let game_id_lc = game_id.to_lowercase();

        Game {
            path: path.to_owned(),
            file_name,
            game_id,
            maker_code: String::new(),
            disc_id: 0,
            title,
            region: Region::Unknown,
            platform,
            format: Format::Dol,
            banner: None,
            title_lc,
            file_name_lc,
            game_id_lc,
            banner_handle: Arc::new(OnceLock::new()),
        }
    }

    pub fn matches_lc(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }

        self.title_lc.contains(query) || self.file_name_lc.contains(query) || self.game_id_lc.contains(query)
    }

    pub fn rehydrate_keys(&mut self) {
        if self.title_lc.is_empty() {
            self.title_lc = self.title.to_lowercase();
        }

        if self.file_name_lc.is_empty() {
            self.file_name_lc = self.file_name.to_lowercase();
        }

        if self.game_id_lc.is_empty() {
            self.game_id_lc = self.game_id.to_lowercase();
        }
    }

    pub fn banner_handle(&self) -> Option<Handle> {
        let banner = self.banner.as_ref()?;
        let handle = self
            .banner_handle
            .get_or_init(|| Handle::from_rgba(banner.width, banner.height, banner.rgba.clone()));
        Some(handle.clone())
    }
}

fn decode_cstring(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
}
