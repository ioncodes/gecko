use std::io;
use std::path::Path;

pub const FILE_ID: u32 = 0x0d01_f1f0;
pub const VERSION_NUMBER: u32 = 6;
pub const MIN_LOADER_VERSION: u32 = 1;

pub const BP_MEM_SIZE: usize = 256;
pub const CP_MEM_SIZE: usize = 256;
pub const XF_MEM_SIZE: usize = 4096;
pub const XF_REGS_SIZE: usize = 88;
pub const TEX_MEM_SIZE: usize = 1024 * 1024;

pub const MEM1_SIZE_RETAIL: u32 = 0x0180_0000;
pub const MEM2_SIZE_RETAIL: u32 = 0x0400_0000;

const FLAG_IS_WII: u32 = 1;
const HEADER_SIZE: usize = 128;
const FRAME_INFO_SIZE: usize = 64;
const MEMORY_UPDATE_SIZE: usize = 24;
const DEFAULT_GAME_ID: &str = "00000000";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemoryUpdateType {
    TextureMap = 0x01,
    XfData = 0x02,
    VertexStream = 0x04,
    Tmem = 0x08,
}

impl MemoryUpdateType {
    fn from_raw(raw: u8) -> Self {
        match raw {
            0x02 => MemoryUpdateType::XfData,
            0x04 => MemoryUpdateType::VertexStream,
            0x08 => MemoryUpdateType::Tmem,
            _ => MemoryUpdateType::TextureMap,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MemoryUpdate {
    pub fifo_position: u32,
    pub address: u32,
    pub kind: MemoryUpdateType,
    pub data: Vec<u8>,
}

#[derive(Clone, Default, Debug)]
pub struct Frame {
    pub fifo_data: Vec<u8>,
    pub fifo_start: u32,
    pub fifo_end: u32,
    pub memory_updates: Vec<MemoryUpdate>,
}

#[derive(Clone)]
pub struct DffFile {
    pub bp_mem: Vec<u32>,
    pub cp_mem: Vec<u32>,
    pub xf_mem: Vec<u32>,
    pub xf_regs: Vec<u32>,
    pub tex_mem: Vec<u8>,
    pub is_wii: bool,
    pub mem1_size: u32,
    pub mem2_size: u32,
    pub version: u32,
    pub game_id: String,
    pub frames: Vec<Frame>,
}

impl Default for DffFile {
    fn default() -> Self {
        DffFile {
            bp_mem: vec![0; BP_MEM_SIZE],
            cp_mem: vec![0; CP_MEM_SIZE],
            xf_mem: vec![0; XF_MEM_SIZE],
            xf_regs: vec![0; XF_REGS_SIZE],
            tex_mem: vec![0; TEX_MEM_SIZE],
            is_wii: false,
            mem1_size: MEM1_SIZE_RETAIL,
            mem2_size: MEM2_SIZE_RETAIL,
            version: VERSION_NUMBER,
            game_id: DEFAULT_GAME_ID.to_string(),
            frames: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub enum DffError {
    Io(io::Error),
    BadMagic(u32),
    UnsupportedVersion(u32),
    Truncated,
}

impl std::fmt::Display for DffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DffError::Io(e) => write!(f, "io error: {e}"),
            DffError::BadMagic(m) => write!(f, "bad DFF magic {m:#010x}, expected {FILE_ID:#010x}"),
            DffError::UnsupportedVersion(v) => {
                write!(f, "DFF min loader version {v} exceeds supported {VERSION_NUMBER}")
            }
            DffError::Truncated => write!(f, "DFF file truncated"),
        }
    }
}

impl std::error::Error for DffError {}

impl From<io::Error> for DffError {
    fn from(e: io::Error) -> Self {
        DffError::Io(e)
    }
}

impl DffFile {
    pub fn save(&self, path: &Path) -> Result<(), DffError> {
        std::fs::write(path, self.to_bytes())?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, DffError> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let frame_list_offset = HEADER_SIZE as u64;
        let mut out = vec![0u8; HEADER_SIZE + self.frames.len() * FRAME_INFO_SIZE];

        let bp_mem_offset = self::append_u32_slice(&mut out, &self.bp_mem, BP_MEM_SIZE);
        let cp_mem_offset = self::append_u32_slice(&mut out, &self.cp_mem, CP_MEM_SIZE);
        let xf_mem_offset = self::append_u32_slice(&mut out, &self.xf_mem, XF_MEM_SIZE);
        let xf_regs_offset = self::append_u32_slice(&mut out, &self.xf_regs, XF_REGS_SIZE);

        let tex_mem_offset = out.len() as u64;
        let n = self.tex_mem.len().min(TEX_MEM_SIZE);
        out.extend_from_slice(&self.tex_mem[..n]);
        out.resize(tex_mem_offset as usize + TEX_MEM_SIZE, 0);

        for (i, frame) in self.frames.iter().enumerate() {
            let fifo_data_offset = out.len() as u64;
            out.extend_from_slice(&frame.fifo_data);

            let updates_offset = out.len() as u64;
            out.resize(out.len() + frame.memory_updates.len() * MEMORY_UPDATE_SIZE, 0);

            for (j, update) in frame.memory_updates.iter().enumerate() {
                let data_offset = out.len() as u64;
                out.extend_from_slice(&update.data);

                let entry = updates_offset as usize + j * MEMORY_UPDATE_SIZE;
                out[entry..entry + 4].copy_from_slice(&update.fifo_position.to_le_bytes());
                out[entry + 4..entry + 8].copy_from_slice(&update.address.to_le_bytes());
                out[entry + 8..entry + 16].copy_from_slice(&data_offset.to_le_bytes());
                out[entry + 16..entry + 20].copy_from_slice(&(update.data.len() as u32).to_le_bytes());
                out[entry + 20] = update.kind as u8;
            }

            let entry = frame_list_offset as usize + i * FRAME_INFO_SIZE;
            out[entry..entry + 8].copy_from_slice(&fifo_data_offset.to_le_bytes());
            out[entry + 8..entry + 12].copy_from_slice(&(frame.fifo_data.len() as u32).to_le_bytes());
            out[entry + 12..entry + 16].copy_from_slice(&frame.fifo_start.to_le_bytes());
            out[entry + 16..entry + 20].copy_from_slice(&frame.fifo_end.to_le_bytes());
            out[entry + 20..entry + 28].copy_from_slice(&updates_offset.to_le_bytes());
            out[entry + 28..entry + 32].copy_from_slice(&(frame.memory_updates.len() as u32).to_le_bytes());
        }

        let mut h = HeaderWriter::new(&mut out);
        h.u32(FILE_ID);
        h.u32(VERSION_NUMBER);
        h.u32(MIN_LOADER_VERSION);
        h.u64(bp_mem_offset);
        h.u32(BP_MEM_SIZE as u32);
        h.u64(cp_mem_offset);
        h.u32(CP_MEM_SIZE as u32);
        h.u64(xf_mem_offset);
        h.u32(XF_MEM_SIZE as u32);
        h.u64(xf_regs_offset);
        h.u32(XF_REGS_SIZE as u32);
        h.u64(frame_list_offset);
        h.u32(self.frames.len() as u32);
        h.u32(if self.is_wii { FLAG_IS_WII } else { 0 });
        h.u64(tex_mem_offset);
        h.u32(TEX_MEM_SIZE as u32);
        h.u32(self.mem1_size);
        h.u32(self.mem2_size);
        h.game_id(&self.game_id);

        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DffError> {
        let mut r = Reader { bytes, pos: 0 };

        let magic = r.u32()?;
        if magic != FILE_ID {
            return Err(DffError::BadMagic(magic));
        }

        let version = r.u32()?;
        let min_loader_version = r.u32()?;
        if min_loader_version > VERSION_NUMBER {
            return Err(DffError::UnsupportedVersion(min_loader_version));
        }

        let bp_mem_offset = r.u64()?;
        let bp_mem_size = r.u32()? as usize;
        let cp_mem_offset = r.u64()?;
        let cp_mem_size = r.u32()? as usize;
        let xf_mem_offset = r.u64()?;
        let xf_mem_size = r.u32()? as usize;
        let xf_regs_offset = r.u64()?;
        let xf_regs_size = r.u32()? as usize;
        let frame_list_offset = r.u64()?;
        let frame_count = r.u32()?;
        let flags = r.u32()?;
        let tex_mem_offset = r.u64()?;
        let tex_mem_size = r.u32()? as usize;
        let mem1_size = r.u32()?;
        let mem2_size = r.u32()?;
        let game_id_raw = r.bytes(8)?;

        let mut file = DffFile {
            is_wii: flags & FLAG_IS_WII != 0,
            version,
            ..DffFile::default()
        };

        if version >= 5 {
            file.mem1_size = mem1_size;
            file.mem2_size = mem2_size;
        }

        if version >= 6 {
            let len = game_id_raw.iter().position(|&b| b == 0).unwrap_or(8);
            file.game_id = String::from_utf8_lossy(&game_id_raw[..len]).into_owned();
        }

        self::read_u32_section(bytes, bp_mem_offset, bp_mem_size, &mut file.bp_mem)?;
        self::read_u32_section(bytes, cp_mem_offset, cp_mem_size, &mut file.cp_mem)?;
        self::read_u32_section(bytes, xf_mem_offset, xf_mem_size, &mut file.xf_mem)?;
        self::read_u32_section(bytes, xf_regs_offset, xf_regs_size, &mut file.xf_regs)?;

        if version >= 4 {
            let n = tex_mem_size.min(TEX_MEM_SIZE);
            let src = self::slice_at(bytes, tex_mem_offset, n)?;
            file.tex_mem[..n].copy_from_slice(src);
        }

        for i in 0..frame_count as usize {
            let entry = frame_list_offset as usize + i * FRAME_INFO_SIZE;
            let mut fr = Reader {
                bytes: self::slice_at(bytes, entry as u64, FRAME_INFO_SIZE)?,
                pos: 0,
            };
            let fifo_data_offset = fr.u64()?;
            let fifo_data_size = fr.u32()? as usize;
            let fifo_start = fr.u32()?;
            let fifo_end = fr.u32()?;
            let updates_offset = fr.u64()?;
            let num_updates = fr.u32()?;

            let mut frame = Frame {
                fifo_data: self::slice_at(bytes, fifo_data_offset, fifo_data_size)?.to_vec(),
                fifo_start,
                fifo_end,
                memory_updates: Vec::with_capacity(num_updates as usize),
            };

            for j in 0..num_updates as usize {
                let entry = updates_offset as usize + j * MEMORY_UPDATE_SIZE;
                let mut ur = Reader {
                    bytes: self::slice_at(bytes, entry as u64, MEMORY_UPDATE_SIZE)?,
                    pos: 0,
                };
                let fifo_position = ur.u32()?;
                let address = ur.u32()?;
                let data_offset = ur.u64()?;
                let data_size = ur.u32()? as usize;
                let kind = MemoryUpdateType::from_raw(ur.bytes(1)?[0]);

                frame.memory_updates.push(MemoryUpdate {
                    fifo_position,
                    address,
                    kind,
                    data: self::slice_at(bytes, data_offset, data_size)?.to_vec(),
                });
            }

            file.frames.push(frame);
        }

        Ok(file)
    }
}

fn append_u32_slice(out: &mut Vec<u8>, values: &[u32], count: usize) -> u64 {
    let offset = out.len() as u64;
    for i in 0..count {
        let v = values.get(i).copied().unwrap_or(0);
        out.extend_from_slice(&v.to_le_bytes());
    }
    offset
}

fn read_u32_section(bytes: &[u8], offset: u64, count: usize, dst: &mut [u32]) -> Result<(), DffError> {
    let n = count.min(dst.len());
    let src = self::slice_at(bytes, offset, n * 4)?;
    for (i, chunk) in src.chunks_exact(4).enumerate() {
        dst[i] = u32::from_le_bytes(chunk.try_into().unwrap());
    }
    Ok(())
}

fn slice_at(bytes: &[u8], offset: u64, len: usize) -> Result<&[u8], DffError> {
    let start = offset as usize;
    bytes.get(start..start + len).ok_or(DffError::Truncated)
}

struct HeaderWriter<'a> {
    out: &'a mut [u8],
    pos: usize,
}

impl<'a> HeaderWriter<'a> {
    fn new(out: &'a mut [u8]) -> Self {
        HeaderWriter { out, pos: 0 }
    }

    fn u32(&mut self, v: u32) {
        self.out[self.pos..self.pos + 4].copy_from_slice(&v.to_le_bytes());
        self.pos += 4;
    }

    fn u64(&mut self, v: u64) {
        self.out[self.pos..self.pos + 8].copy_from_slice(&v.to_le_bytes());
        self.pos += 8;
    }

    fn game_id(&mut self, id: &str) {
        let id = if id.len() > 8 || !id.is_ascii() {
            DEFAULT_GAME_ID
        } else {
            id
        };

        let bytes = id.as_bytes();
        self.out[self.pos..self.pos + bytes.len()].copy_from_slice(bytes);
        self.pos += 8;
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], DffError> {
        let s = self.bytes.get(self.pos..self.pos + n).ok_or(DffError::Truncated)?;
        self.pos += n;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, DffError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, DffError> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
}
