use crate::system::{System, SystemId, WII};
use std::path::PathBuf;

pub const STATE_MAGIC: [u8; 4] = *b"GKST";
pub const STATE_VERSION: u32 = 1;

const COMPRESSION_LEVEL: i32 = 1;

const HEADER_LEN: usize = size_of::<[u8; 4]>() + size_of::<u32>() + size_of::<SystemId>() + size_of::<u64>();

#[derive(Debug)]
pub enum StateError {
    Io(std::io::Error),
    Corrupt(&'static str),
    UnsupportedVersion(u32),
    WrongSystem,
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateError::Io(err) => write!(f, "io error: {err}"),
            StateError::Corrupt(what) => write!(f, "corrupt savestate: {what}"),
            StateError::UnsupportedVersion(v) => write!(f, "unsupported savestate version {v}"),
            StateError::WrongSystem => write!(f, "savestate was taken on a different system type"),
        }
    }
}

impl std::error::Error for StateError {}

impl From<std::io::Error> for StateError {
    fn from(err: std::io::Error) -> Self {
        StateError::Io(err)
    }
}

pub struct StateWriter {
    buf: Vec<u8>,
}

impl StateWriter {
    pub fn new() -> Self {
        Self::with_capacity(1 << 20)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        StateWriter {
            buf: Vec::with_capacity(capacity),
        }
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }

    #[inline(always)]
    pub fn pod<T: Copy>(&mut self, value: &T) {
        let bytes = unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) };
        self.buf.extend_from_slice(bytes);
    }

    #[inline(always)]
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    #[inline(always)]
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(always)]
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(always)]
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(always)]
    pub fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(always)]
    pub fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(always)]
    pub fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }

    #[inline(always)]
    pub fn bytes(&mut self, b: &[u8]) {
        self.u64(b.len() as u64);
        self.buf.extend_from_slice(b);
    }

    #[inline(always)]
    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
}

pub struct StateReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> StateReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        StateReader { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    #[inline(always)]
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], StateError> {
        if self.remaining() < n {
            return Err(StateError::Corrupt("unexpected end of data"));
        }

        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    #[inline(always)]
    pub fn pod<T: Copy>(&mut self) -> Result<T, StateError> {
        let bytes = self.take(size_of::<T>())?;
        Ok(unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<T>()) })
    }

    #[inline(always)]
    pub fn u8(&mut self) -> Result<u8, StateError> {
        Ok(self.take(1)?[0])
    }

    #[inline(always)]
    pub fn u16(&mut self) -> Result<u16, StateError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    #[inline(always)]
    pub fn u32(&mut self) -> Result<u32, StateError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    #[inline(always)]
    pub fn u64(&mut self) -> Result<u64, StateError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    #[inline(always)]
    pub fn i32(&mut self) -> Result<i32, StateError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    #[inline(always)]
    pub fn i64(&mut self) -> Result<i64, StateError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    #[inline(always)]
    pub fn bool(&mut self) -> Result<bool, StateError> {
        Ok(self.u8()? != 0)
    }

    #[inline(always)]
    pub fn bytes(&mut self) -> Result<&'a [u8], StateError> {
        let len = usize::try_from(self.u64()?).map_err(|_| StateError::Corrupt("buffer size exceeds address space"))?;

        self.take(len)
    }

    pub fn str(&mut self) -> Result<String, StateError> {
        let bytes = self.bytes()?;
        String::from_utf8(bytes.to_vec()).map_err(|_| StateError::Corrupt("invalid utf-8 string"))
    }

    pub fn bytes_into(&mut self, dst: &mut [u8]) -> Result<(), StateError> {
        let bytes = self.bytes()?;
        if bytes.len() != dst.len() {
            return Err(StateError::Corrupt(
                "buffer size mismatch (different boot configuration?)",
            ));
        }

        dst.copy_from_slice(bytes);
        Ok(())
    }
}

pub fn pack(system: SystemId, payload: &[u8]) -> Result<Vec<u8>, StateError> {
    let compressed = zstd::bulk::compress(payload, COMPRESSION_LEVEL)?;

    let mut out = Vec::with_capacity(compressed.len() + HEADER_LEN);
    out.extend_from_slice(&STATE_MAGIC);
    out.extend_from_slice(&STATE_VERSION.to_le_bytes());
    out.push(system);
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

pub fn unpack(system: SystemId, data: &[u8]) -> Result<Vec<u8>, StateError> {
    if data.len() < HEADER_LEN {
        return Err(StateError::Corrupt("file too short"));
    }

    if data[0..4] != STATE_MAGIC {
        return Err(StateError::Corrupt("bad magic"));
    }

    let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if version != STATE_VERSION {
        return Err(StateError::UnsupportedVersion(version));
    }

    if data[8] != system {
        return Err(StateError::WrongSystem);
    }

    let uncompressed_len = u64::from_le_bytes(data[9..17].try_into().unwrap());
    if uncompressed_len > (1u64 << 32) {
        return Err(StateError::Corrupt("implausible payload size"));
    }

    let uncompressed_len =
        usize::try_from(uncompressed_len).map_err(|_| StateError::Corrupt("payload size exceeds address space"))?;
    let payload = zstd::bulk::decompress(&data[HEADER_LEN..], uncompressed_len)?;

    Ok(payload)
}

impl<const SYSTEM: SystemId> System<SYSTEM> {
    pub fn save_state(&mut self) -> Result<Vec<u8>, StateError> {
        let mut ram = self.mmio.ram_view_mut();
        self.render_sink.flush_efb_copies(&mut ram);
        self.mmio.clear_deferred_efb_writebacks();

        let mut w = StateWriter::new();

        w.bool(self.vsync_pending);
        w.bool(self.vi_present_seen_this_frame);

        self.scheduler.save_state(&mut w);

        w.pod(&self.gekko);
        self.mmio.save_state(&mut w);

        w.pod(&self.vi);
        w.pod(&self.pe);
        w.pod(&self.pi);
        w.pod(&self.mi);
        w.pod(&self.si);
        w.pod(&self.ai);
        w.pod(&self.cp);

        self.di.save_state(&mut w);
        self.dsp.save_state(&mut w);
        self.gx.save_state(&mut w);
        self.exi.save_state(&mut w);

        if SYSTEM == WII {
            self.starlet.save_state(&mut w);
            w.pod(&self.hollywood);
        }

        self::pack(SYSTEM, &w.into_inner())
    }

    pub fn load_state(&mut self, data: &[u8]) -> Result<(), StateError> {
        self.mmio.clear_deferred_efb_writebacks();
        let payload = self::unpack(SYSTEM, data)?;
        let mut r = StateReader::new(&payload);

        self.vsync_pending = r.bool()?;
        self.vi_present_seen_this_frame = r.bool()?;

        self.scheduler.load_state(&mut r)?;

        self.gekko = r.pod()?;
        self.mmio.load_state(&mut r)?;

        self.vi = r.pod()?;
        self.pe = r.pod()?;
        self.pi = r.pod()?;
        self.mi = r.pod()?;
        self.si = r.pod()?;
        self.ai = r.pod()?;
        self.cp = r.pod()?;

        self.di.load_state(&mut r)?;
        self.dsp.load_state(&mut r)?;
        self.gx.load_state(&mut r)?;
        self.exi.load_state(&mut r)?;

        if SYSTEM == WII {
            self.starlet.load_state(&mut r)?;
            self.hollywood = r.pod()?;
        }

        if r.remaining() != 0 {
            return Err(StateError::Corrupt("trailing data"));
        }

        #[cfg(feature = "jit")]
        match self.jit.as_mut() {
            Some(jit) => jit.flush_with_refcount(&mut self.mmio),
            None => self.mmio.clear_code_refcount(),
        }

        #[cfg(not(feature = "jit"))]
        self.mmio.clear_code_refcount();

        self.render_sink.exec(crate::host::GxAction::InvalidateCaches);
        self.render_sink.reset_efb();

        Ok(())
    }

    pub fn save_state_to_file(&mut self, path: &std::path::Path) -> Result<(), StateError> {
        let data = self.save_state()?;
        Ok(self::write_state_file(path, &data)?)
    }

    pub fn load_state_from_file(&mut self, path: &std::path::Path) -> Result<(), StateError> {
        let data = std::fs::read(path)?;
        self.load_state(&data)
    }
}

pub fn state_path(game_id: Option<&str>) -> PathBuf {
    crate::jit::cache::cache_dir(game_id.unwrap_or("default")).join("state.gkst")
}

pub fn write_state_file(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("gkst.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}
