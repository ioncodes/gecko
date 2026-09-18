use std::path::Path;

const BASE_DIRS: [&str; 9] = [
    "sys", "ticket", "title", "shared1", "shared2", "tmp", "import", "meta", "wfs",
];

const SETTING_TXT_PATH: &str = "title/00000001/00000002/data/setting.txt";
const SETTING_SEED: u32 = 0x73B5_DBFA;
const SERIAL_NUMBER: &str = "696969420";

pub const WC24_CONFIG_PATH: &str = "shared2/wc24/nwc24msg.cfg";
const WC24_DIR: &str = "shared2/wc24";

pub fn ensure_skeleton(root: &Path) {
    for dir in BASE_DIRS {
        let path = root.join(dir);
        if let Err(err) = std::fs::create_dir_all(&path) {
            tracing::warn!(path = %path.display(), %err, "NAND: create dir failed");
        }
    }
}

pub fn sync_setting_txt(root: &Path, game_code: [u8; 4]) {
    let path = root.join(SETTING_TXT_PATH);
    let setting = RegionSetting::from_game_code(game_code);

    if let Ok(existing) = std::fs::read(&path) {
        match self::decode_area(&existing) {
            Some(area) if area == setting.area => return,
            Some(area) => tracing::info!(
                old = %area,
                new = setting.area,
                "NAND: setting.txt region mismatch, regenerating for disc"
            ),
            None => tracing::warn!("NAND: undecodable setting.txt, regenerating"),
        }
    }

    self::write_new(&path, &setting.encode());
}

fn decode_area(data: &[u8]) -> Option<String> {
    let mut key = SETTING_SEED;

    let decoded: Vec<u8> = data
        .iter()
        .take(0x100)
        .map(|&b| {
            let plain = b ^ (key as u8);
            key = key.rotate_left(1);
            plain
        })
        .collect();

    let text = String::from_utf8_lossy(&decoded);
    text.lines()
        .find_map(|line| line.strip_prefix("AREA="))
        .map(|value| value.trim_end().to_owned())
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

// https://github.com/kiwi515/ogws/tree/master/include/revolution/NWC24/internal
pub fn ensure_wc24_files(root: &Path) {
    let cfg = root.join(WC24_CONFIG_PATH);
    let cbk = cfg.with_extension("cbk");

    if !cfg.exists() || !cbk.exists() {
        let msg = std::fs::read(&cfg)
            .or_else(|_| std::fs::read(&cbk))
            .unwrap_or_else(|_| self::wc24_msg_cfg());
        for path in [&cfg, &cbk] {
            if !path.exists() {
                self::write_new(path, &msg);
            }
        }
    }

    let dir = root.join(WC24_DIR);
    let files: [(&str, fn() -> Vec<u8>); 7] = [
        ("nwc24fl.bin", || {
            // NWC24iFLHeader: magic "WcFl", version, capacity
            self::be_words(0x8060, &[0x5763466C, 2, 100])
        }),
        ("nwc24fls.bin", || {
            // NWC24iSecretFLHeader: magic "WcFs", version, undocumented +0x08 default
            self::be_words(0x3200, &[0x57634673, 2, 0x150])
        }),
        ("nwc24dl.bin", self::wc24_dl),
        ("mbox/wc24send.ctl", || self::wc24_ctl(0x4000, 127, 0x200000)),
        ("mbox/wc24recv.ctl", || self::wc24_ctl(0x8000, 255, 0x700000)),
        ("mbox/wc24send.mbx", || self::wc24_mbx(0x200000)),
        ("mbox/wc24recv.mbx", || self::wc24_mbx(0x700000)),
    ];

    for (name, build) in files {
        let path = dir.join(name);
        if !path.exists() {
            self::write_new(&path, &build());
        }
    }
}

fn put_u32(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn be_words(size: usize, words: &[u32]) -> Vec<u8> {
    let mut data = vec![0; size];
    for (i, &word) in words.iter().enumerate() {
        self::put_u32(&mut data, i * 4, word);
    }
    data
}

fn wc24_msg_cfg() -> Vec<u8> {
    // https://github.com/kiwi515/ogws/blob/master/include/revolution/NWC24/internal/NWC24iConfig.h
    // https://wiibrew.org/wiki//shared2/wc24/nwc24msg.cfg
    let header = [
        0x57634366, // +0x00: "WcCf" magic
        8,          // +0x04: format version
        0,          // +0x08: dummy user ID, high word
        0x69696,    // +0x0C: dummy user ID, low word
        1,          // +0x10: ID creation counter
        1,          // +0x14: creation stage
    ];
    let mut data = self::be_words(0x400, &header); // 1 KiB config, including checksum
    // https://github.com/kiwi515/ogws/blob/master/src/revolution/NWC24/NWC24Config.c
    let checksum = header.iter().fold(0u32, |sum, &v| sum.wrapping_add(v));
    self::put_u32(&mut data, 0x3FC, checksum);
    data
}

fn wc24_dl() -> Vec<u8> {
    // https://github.com/kiwi515/ogws/blob/master/include/revolution/NWC24/internal/NWC24iDownload.h
    // https://wiibrew.org/wiki//shared2/wc24/nwc24dl.bin
    let header = [
        0x5763446C, // +0x00: "WcDl" magic
        1,          // +0x04: format version
        0,          // +0x08: unknown/reserved
        0,          // +0x0C: unknown/reserved
        0x00200008, // +0x10: u16 max subtasks (32), u16 private tasks (8)
        120 << 16,  // +0x14: u16 task capacity (120), then two zero bytes
    ];
    let mut data = self::be_words(0xF800, &header);
    for i in 0..120u32 {
        self::put_u32(&mut data, 0x800 + i as usize * 0x200, (i << 16) | 0xFF00);
    }
    data
}

fn wc24_ctl(size: usize, max_entries: u32, mbx_size: u32) -> Vec<u8> {
    // https://github.com/kiwi515/ogws/blob/master/include/revolution/NWC24/internal/NWC24iMBoxCtrl.h
    let mut data = self::be_words(
        size,
        &[
            0x57635466,  // +0x00: "WcTf" magic
            4,           // +0x04: format version
            0,           // +0x08: current message count
            max_entries, // +0x0C: message capacity
            0,           // +0x10: total stored message bytes
            size as u32, // +0x14: control file size in bytes
            1,           // +0x18: next message ID
            128,         // +0x1C: first free entry, immediately after the 0x80-byte header
            0,           // +0x20: oldest message ID (none)
            mbx_size,    // +0x24: initial mailbox free-space budget
        ],
    );
    // Thx Dolphin
    data[0x58..0x7F].fill(b'0');
    for offset in (0x80..size - 0x80).step_by(0x80) {
        self::put_u32(&mut data, offset + 12, (offset + 0x80) as u32);
    }
    data
}

fn wc24_mbx(size: usize) -> Vec<u8> {
    // https://wiibrew.org/wiki/VFF
    let mut data = self::be_words(
        size,
        &[
            0x56464620,  // +0x00: "VFF " magic
            0xFEFF0100,  // +0x04: ??
            size as u32, // +0x08: total volume size in bytes
            0x00200000,  // +0x0C: u16 header size (32 bytes), then zero padding
        ],
    );
    let fat_size = (size / 512 * 2 + 511) & !511;
    for offset in [32, 32 + fat_size] {
        self::put_u32(&mut data, offset, 0xF0FFFFFF);
    }
    data
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

pub(crate) struct RegionSetting {
    area: &'static str,
    video: &'static str,
    code: &'static str,
    game: &'static str,
}

impl RegionSetting {
    pub(crate) fn language(&self) -> u8 {
        match self.area {
            "JPN" => 0,
            "KOR" => 9,
            _ => 1,
        }
    }

    pub(crate) fn from_game_code(game_code: [u8; 4]) -> Self {
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
