use std::path::Path;

const BASE_DIRS: [&str; 9] = [
    "sys", "ticket", "title", "shared1", "shared2", "tmp", "import", "meta", "wfs",
];

const SETTING_TXT_PATH: &str = "title/00000001/00000002/data/setting.txt";
const SETTING_SEED: u32 = 0x73B5_DBFA;
const SERIAL_NUMBER: &str = "696969420";

pub fn ensure_skeleton(root: &Path) {
    for dir in BASE_DIRS {
        let path = root.join(dir);
        if let Err(err) = std::fs::create_dir_all(&path) {
            tracing::warn!(path = %path.display(), %err, "NAND: create dir failed");
        }
    }
}

pub fn ensure_setting_txt(root: &Path, game_code: [u8; 4]) {
    let path = root.join(SETTING_TXT_PATH);
    if path.exists() {
        return;
    }

    self::write_new(&path, &RegionSetting::from_game_code(game_code).encode());
}

pub fn ensure_title_dirs(root: &Path, title_id: u64) {
    let title_dir = root.join(format!("title/{:08x}/{:08x}", (title_id >> 32) as u32, title_id as u32));

    for sub in ["content", "data"] {
        let path = title_dir.join(sub);
        if let Err(err) = std::fs::create_dir_all(&path) {
            tracing::warn!(path = %path.display(), %err, "NAND: create title dir failed");
        }
    }
}

pub fn write_new(path: &Path, data: &[u8]) {
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            tracing::warn!(path = %parent.display(), %err, "NAND: create parent failed");
            return;
        }
    }

    match std::fs::write(path, data) {
        Ok(()) => tracing::info!(path = %path.display(), "NAND: generated default"),
        Err(err) => tracing::warn!(path = %path.display(), %err, "NAND: write failed"),
    }
}

struct RegionSetting {
    area: &'static str,
    video: &'static str,
    code: &'static str,
    game: &'static str,
}

impl RegionSetting {
    fn from_game_code(game_code: [u8; 4]) -> Self {
        match game_code[3] {
            b'J' => Self {
                area: "JPN",
                video: "NTSC",
                code: "JP",
                game: "LJH",
            },
            b'P' | b'D' | b'F' | b'S' | b'I' | b'H' | b'U' | b'X' | b'Y' | b'Z' => Self {
                area: "EUR",
                video: "PAL",
                code: "EU",
                game: "LEH",
            },
            b'K' | b'Q' | b'T' => Self {
                area: "KOR",
                video: "NTSC",
                code: "KR",
                game: "LKH",
            },
            _ => Self {
                area: "USA",
                video: "NTSC",
                code: "US",
                game: "LU",
            },
        }
    }

    fn encode(&self) -> Vec<u8> {
        let model = format!("RVL-001({})", self.area);
        let mut writer = SettingWriter::new();
        writer.add("AREA", self.area);
        writer.add("MODEL", &model);
        writer.add("DVD", "0");
        writer.add("MPCH", "0x7FFE");
        writer.add("CODE", self.code);
        writer.add("SERNO", SERIAL_NUMBER);
        writer.add("VIDEO", self.video);
        writer.add("GAME", self.game);
        writer.finish()
    }
}

struct SettingWriter {
    buffer: [u8; 0x100],
    position: usize,
    key: u32,
}

impl SettingWriter {
    fn new() -> Self {
        Self {
            buffer: [0u8; 0x100],
            position: 0,
            key: SETTING_SEED,
        }
    }

    fn add(&mut self, key: &str, value: &str) {
        self.write_line(&format!("{key}={value}\r\n"));
    }

    fn write_line(&mut self, line: &str) {
        loop {
            let old_position = self.position;
            let old_key = self.key;
            for b in line.bytes() {
                self.write_byte(b);
            }

            if !self.buffer[old_position..self.position].contains(&0) {
                return;
            }

            self.position = old_position;
            self.key = old_key;
            self.write_byte(b'\n');
        }
    }

    fn write_byte(&mut self, b: u8) {
        if self.position >= self.buffer.len() {
            return;
        }

        self.buffer[self.position] = b ^ (self.key as u8);
        self.position += 1;
        self.key = self.key.rotate_left(1);
    }

    fn finish(self) -> Vec<u8> {
        self.buffer.to_vec()
    }
}
