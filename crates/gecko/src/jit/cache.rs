use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAGIC: [u8; 4] = *b"GBLK";

pub const JIT_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum CachedSystem {
    Gc = 0,
    Wii = 1,
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum CachedKind {
    Ppc = 0,
    Dsp = 1,
    Vtx = 2,
}

#[derive(Clone, Copy, Debug)]
pub struct CachedBlockPpc {
    pub pc: u32,
    pub instr_count: u16,
    pub hash: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct CachedBlockDsp {
    pub pc: u16,
    pub instr_count: u16,
    pub hash: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct CachedVtxKey {
    pub vcd_lo: u32,
    pub vcd_hi: u32,
    pub vat_a: u32,
    pub vat_b: u32,
    pub vat_c: u32,
}

#[inline]
pub fn hash_words(words: impl IntoIterator<Item = u32>) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for w in words {
        h ^= w as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn cache_dir(game_id: &str) -> PathBuf {
    PathBuf::from("cache").join(game_id)
}

pub fn ppc_cache_path(game_id: &str) -> PathBuf {
    self::cache_dir(game_id).join("ppc.bin")
}

pub fn dsp_cache_path(game_id: &str) -> PathBuf {
    self::cache_dir(game_id).join("dsp.bin")
}

pub fn vtx_cache_path(game_id: &str) -> PathBuf {
    self::cache_dir(game_id).join("vtx.bin")
}

fn write_header<W: Write>(writer: &mut W, kind: CachedKind, system: CachedSystem, count: u32) -> std::io::Result<()> {
    writer.write_all(&MAGIC)?;
    writer.write_all(&JIT_VERSION.to_le_bytes())?;
    writer.write_all(&[kind as u8, system as u8, 0, 0])?;
    writer.write_all(&count.to_le_bytes())
}

fn read_header<R: Read>(reader: &mut R) -> std::io::Result<(CachedKind, CachedSystem, u32)> {
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;

    if magic != MAGIC {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "bad jit cache magic",
        ));
    }

    let mut version_bytes = [0u8; 4];
    reader.read_exact(&mut version_bytes)?;
    let version = u32::from_le_bytes(version_bytes);

    if version != JIT_VERSION {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "jit cache version mismatch",
        ));
    }

    let mut tag = [0u8; 4];
    reader.read_exact(&mut tag)?;
    let kind = match tag[0] {
        0 => CachedKind::Ppc,
        1 => CachedKind::Dsp,
        2 => CachedKind::Vtx,
        _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "bad kind tag")),
    };

    let system = match tag[1] {
        0 => CachedSystem::Gc,
        1 => CachedSystem::Wii,
        _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "bad system tag")),
    };

    let mut count_bytes = [0u8; 4];
    reader.read_exact(&mut count_bytes)?;
    let count = u32::from_le_bytes(count_bytes);

    Ok((kind, system, count))
}

pub fn save_ppc_blocks(path: &Path, system: CachedSystem, blocks: &[CachedBlockPpc]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        self::write_header(&mut file, CachedKind::Ppc, system, blocks.len() as u32)?;
        for b in blocks {
            file.write_all(&b.pc.to_le_bytes())?;
            file.write_all(&b.instr_count.to_le_bytes())?;
            file.write_all(&[0u8; 2])?;
            file.write_all(&b.hash.to_le_bytes())?;
        }
        file.sync_data().ok();
    }

    std::fs::rename(&tmp, path)
}

pub fn load_ppc_blocks(path: &Path) -> std::io::Result<Vec<CachedBlockPpc>> {
    let mut file = std::fs::File::open(path)?;
    let (kind, _system, count) = self::read_header(&mut file)?;
    if !matches!(kind, CachedKind::Ppc) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "kind mismatch"));
    }

    let mut blocks = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut pc_bytes = [0u8; 4];
        file.read_exact(&mut pc_bytes)?;

        let mut count_bytes = [0u8; 2];
        file.read_exact(&mut count_bytes)?;

        let mut reserved = [0u8; 2];
        file.read_exact(&mut reserved)?;

        let mut hash_bytes = [0u8; 8];
        file.read_exact(&mut hash_bytes)?;

        blocks.push(CachedBlockPpc {
            pc: u32::from_le_bytes(pc_bytes),
            instr_count: u16::from_le_bytes(count_bytes),
            hash: u64::from_le_bytes(hash_bytes),
        });
    }

    Ok(blocks)
}

pub fn save_dsp_blocks(path: &Path, system: CachedSystem, blocks: &[CachedBlockDsp]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        self::write_header(&mut file, CachedKind::Dsp, system, blocks.len() as u32)?;
        for b in blocks {
            file.write_all(&b.pc.to_le_bytes())?;
            file.write_all(&b.instr_count.to_le_bytes())?;
            file.write_all(&b.hash.to_le_bytes())?;
        }
        file.sync_data().ok();
    }

    std::fs::rename(&tmp, path)
}

pub fn load_dsp_blocks(path: &Path) -> std::io::Result<Vec<CachedBlockDsp>> {
    let mut file = std::fs::File::open(path)?;
    let (kind, _system, count) = self::read_header(&mut file)?;
    if !matches!(kind, CachedKind::Dsp) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "kind mismatch"));
    }

    let mut blocks = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut pc_bytes = [0u8; 2];
        file.read_exact(&mut pc_bytes)?;

        let mut count_bytes = [0u8; 2];
        file.read_exact(&mut count_bytes)?;

        let mut hash_bytes = [0u8; 8];
        file.read_exact(&mut hash_bytes)?;

        blocks.push(CachedBlockDsp {
            pc: u16::from_le_bytes(pc_bytes),
            instr_count: u16::from_le_bytes(count_bytes),
            hash: u64::from_le_bytes(hash_bytes),
        });
    }

    Ok(blocks)
}

pub fn save_vtx_keys(path: &Path, system: CachedSystem, keys: &[CachedVtxKey]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        self::write_header(&mut file, CachedKind::Vtx, system, keys.len() as u32)?;

        for k in keys {
            file.write_all(&k.vcd_lo.to_le_bytes())?;
            file.write_all(&k.vcd_hi.to_le_bytes())?;
            file.write_all(&k.vat_a.to_le_bytes())?;
            file.write_all(&k.vat_b.to_le_bytes())?;
            file.write_all(&k.vat_c.to_le_bytes())?;
        }

        file.sync_data().ok();
    }

    std::fs::rename(&tmp, path)
}

pub fn load_vtx_keys(path: &Path) -> std::io::Result<Vec<CachedVtxKey>> {
    let mut file = std::fs::File::open(path)?;

    let (kind, _system, count) = self::read_header(&mut file)?;
    if !matches!(kind, CachedKind::Vtx) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "kind mismatch"));
    }

    let mut keys = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut buf = [0u8; 4];
        let read_word = |f: &mut std::fs::File, b: &mut [u8; 4]| -> std::io::Result<u32> {
            f.read_exact(b)?;
            Ok(u32::from_le_bytes(*b))
        };

        let vcd_lo = read_word(&mut file, &mut buf)?;
        let vcd_hi = read_word(&mut file, &mut buf)?;
        let vat_a = read_word(&mut file, &mut buf)?;
        let vat_b = read_word(&mut file, &mut buf)?;
        let vat_c = read_word(&mut file, &mut buf)?;

        keys.push(CachedVtxKey {
            vcd_lo,
            vcd_hi,
            vat_a,
            vat_b,
            vat_c,
        });
    }

    Ok(keys)
}
