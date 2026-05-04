use chapa::BitEnum;

use crate::flipper::dsp::Dsp;
use crate::mmio::{RamView, RamViewMut};
use crate::system::{SystemId, WII};

const ARAM_MASK: usize = 16 * 1024 * 1024 - 1;

const START_END_ADDRESS_MASK: u32 = 0x3FFF_FFFF;
const CURRENT_ADDRESS_MASK: u32 = 0xBFFF_FFFF;

const IFX_COEFS_BASE: u16 = 0xFFA0;

#[derive(BitEnum, Debug, PartialEq, Eq)]
pub enum SampleSize {
    Bits4 = 0,
    Bits8 = 1,
    Bits16 = 2,
    Invalid = 3,
}

#[derive(BitEnum, Debug, PartialEq, Eq)]
pub enum SampleMode {
    Adpcm = 0,
    MmioPcmNoInc = 1,
    Pcm = 2,
    MmioPcmInc = 3,
}

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct SampleFormat {
    #[bits(0..=1)]
    pub size: SampleSize,
    #[bits(2..=3)]
    pub mode: SampleMode,
    #[bits(4..=5)]
    pub gain_shift_code: u8,
}

impl SampleFormat {
    #[inline(always)]
    pub fn gain_shift(&self) -> u8 {
        match self.gain_shift_code() {
            0 => 11,
            1 => 0,
            2 => 16,
            _ => 11,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Accelerator {
    pub start_addr: u32,
    pub end_addr: u32,
    pub current_addr: u32,
    pub format: SampleFormat,
    pub gain: i16,
    pub yn1: i16,
    pub yn2: i16,
    pub pred_scale: u16,
    pub input: u16,
    pub reads_stopped: bool,
}

impl Accelerator {
    pub fn new() -> Self {
        Self {
            start_addr: 0,
            end_addr: 0,
            current_addr: 0,
            format: SampleFormat::from_raw(0),
            gain: 0,
            yn1: 0,
            yn2: 0,
            pred_scale: 0,
            input: 0,
            reads_stopped: false,
        }
    }

    pub fn set_start_addr(&mut self, v: u32) {
        self.start_addr = v & START_END_ADDRESS_MASK;
    }

    pub fn set_end_addr(&mut self, v: u32) {
        self.end_addr = v & START_END_ADDRESS_MASK;
    }

    pub fn set_current_addr(&mut self, v: u32) {
        self.current_addr = v & CURRENT_ADDRESS_MASK;
    }

    pub fn set_pred_scale(&mut self, v: u16) {
        self.pred_scale = v & 0x7F;
    }

    pub fn set_yn2(&mut self, v: i16) {
        self.yn2 = v;
        self.reads_stopped = false;
    }
}

fn fetch_sample_word<const SYSTEM: SystemId>(dsp: &Dsp, ram: &RamView<'_>) -> u16 {
    let addr = dsp.accelerator.current_addr & 0x7FFF_FFFF;

    match dsp.accelerator.format.size() {
        SampleSize::Bits4 => {
            let byte = self::read_source_byte::<SYSTEM>(dsp, ram, addr >> 1);
            if addr & 1 == 1 {
                (byte & 0xF) as u16
            } else {
                ((byte >> 4) & 0xF) as u16
            }
        }
        SampleSize::Bits8 => self::read_source_byte::<SYSTEM>(dsp, ram, addr) as u16,
        SampleSize::Bits16 => {
            let off = addr.wrapping_mul(2);
            ((self::read_source_byte::<SYSTEM>(dsp, ram, off) as u16) << 8)
                | (self::read_source_byte::<SYSTEM>(dsp, ram, off.wrapping_add(1)) as u16)
        }
        SampleSize::Invalid => 0,
    }
}

#[inline(always)]
fn read_source_byte<const SYSTEM: SystemId>(dsp: &Dsp, ram: &RamView<'_>, byte_addr: u32) -> u8 {
    if SYSTEM == WII {
        if let Some(bytes) = ram.slice(byte_addr as usize, 1) {
            return bytes[0];
        }
    }

    dsp.aram[byte_addr as usize & ARAM_MASK]
}

#[inline(always)]
fn write_source_byte<const SYSTEM: SystemId>(dsp: &mut Dsp, ram: &mut RamViewMut<'_>, byte_addr: u32, value: u8) {
    if SYSTEM == WII {
        if let Some(bytes) = ram.slice_mut(byte_addr as usize, 1) {
            bytes[0] = value;
            return;
        }
    }

    dsp.aram[byte_addr as usize & ARAM_MASK] = value;
}

#[inline(always)]
fn read_coef_pair(dsp: &Dsp, coef_idx: usize) -> (i32, i32) {
    let base = (IFX_COEFS_BASE - 0xFF00) as usize * 2 + coef_idx * 4;
    let c1 = i16::from_be_bytes([dsp.ifx[base], dsp.ifx[base + 1]]) as i32;
    let c2 = i16::from_be_bytes([dsp.ifx[base + 2], dsp.ifx[base + 3]]) as i32;
    (c1, c2)
}

pub fn read_decoded_sample<const SYSTEM: SystemId>(dsp: &mut Dsp, ram: RamView<'_>) -> u16 {
    if dsp.accelerator.reads_stopped {
        return 0;
    }

    let mode = dsp.accelerator.format.mode();

    let raw_sample: i32 = match mode {
        SampleMode::MmioPcmNoInc | SampleMode::MmioPcmInc => dsp.accelerator.input as i16 as i32,
        SampleMode::Adpcm | SampleMode::Pcm => self::fetch_sample_word::<SYSTEM>(dsp, &ram) as i32,
    };

    let coef_idx = ((dsp.accelerator.pred_scale >> 4) & 0x7) as usize;
    let (coef1, coef2) = read_coef_pair(dsp, coef_idx);

    let (val, step_size): (i16, u32) = match mode {
        SampleMode::Adpcm => decode_adpcm::<SYSTEM>(dsp, &ram, raw_sample, coef1, coef2),
        SampleMode::MmioPcmNoInc | SampleMode::Pcm | SampleMode::MmioPcmInc => {
            decode_pcm(dsp, raw_sample, coef1, coef2, mode)
        }
    };

    if dsp.accelerator.current_addr == dsp.accelerator.end_addr.wrapping_add(step_size).wrapping_sub(1) {
        dsp.accelerator.current_addr = dsp.accelerator.start_addr;
        dsp.accelerator.reads_stopped = true;
        // TODO: raise DSP exception?
    }

    dsp.accelerator.set_current_addr(dsp.accelerator.current_addr);
    val as u16
}

fn decode_adpcm<const SYSTEM: SystemId>(
    dsp: &mut Dsp,
    ram: &RamView<'_>,
    raw_sample: i32,
    coef1: i32,
    coef2: i32,
) -> (i16, u32) {
    let mut nibble = raw_sample & 0xF;
    if nibble >= 8 {
        nibble -= 16;
    }

    let scale = 1i32 << (dsp.accelerator.pred_scale & 0xF);

    let val32 =
        scale * nibble + ((0x400 + coef1 * dsp.accelerator.yn1 as i32 + coef2 * dsp.accelerator.yn2 as i32) >> 11);
    let val = val32.clamp(-0x7FFF, 0x7FFF) as i16;

    dsp.accelerator.yn2 = dsp.accelerator.yn1;
    dsp.accelerator.yn1 = val;
    dsp.accelerator.current_addr = dsp.accelerator.current_addr.wrapping_add(1);

    let mut step_size: u32 = 2;
    if (dsp.accelerator.end_addr & 0xF) == 0x0 && dsp.accelerator.current_addr == dsp.accelerator.end_addr {
        dsp.accelerator.current_addr = dsp.accelerator.start_addr.wrapping_add(1);
    } else if (dsp.accelerator.end_addr & 0xF) == 0x1
        && dsp.accelerator.current_addr == dsp.accelerator.end_addr.wrapping_sub(1)
    {
        dsp.accelerator.current_addr = dsp.accelerator.start_addr;
    } else if dsp.accelerator.current_addr & 15 == 0 {
        let header_addr = dsp.accelerator.current_addr & !15;
        let byte = read_source_byte::<SYSTEM>(dsp, ram, header_addr >> 1);
        dsp.accelerator.pred_scale = byte as u16;
        dsp.accelerator.current_addr = dsp.accelerator.current_addr.wrapping_add(2);
        step_size = step_size.wrapping_add(2);
    }

    (val, step_size)
}

fn decode_pcm(dsp: &mut Dsp, raw_sample: i32, coef1: i32, coef2: i32, mode: SampleMode) -> (i16, u32) {
    let gain_shift = dsp.accelerator.format.gain_shift() as i32;

    let gain = dsp.accelerator.gain as i32;
    let yn1 = dsp.accelerator.yn1 as i32;
    let yn2 = dsp.accelerator.yn2 as i32;
    let val32 = ((gain * raw_sample) >> gain_shift) + ((coef1 * yn1) >> gain_shift) + ((coef2 * yn2) >> gain_shift);
    let val = val32 as i16;

    dsp.accelerator.yn2 = dsp.accelerator.yn1;
    dsp.accelerator.yn1 = val;

    if mode != SampleMode::MmioPcmNoInc {
        dsp.accelerator.current_addr = dsp.accelerator.current_addr.wrapping_add(1);
    }

    (val, 2)
}

pub fn read_raw<const SYSTEM: SystemId>(dsp: &mut Dsp, ram: RamView<'_>) -> u16 {
    let val = self::fetch_sample_word::<SYSTEM>(dsp, &ram);

    if dsp.accelerator.format.size() != SampleSize::Invalid {
        dsp.accelerator.current_addr = dsp.accelerator.current_addr.wrapping_add(1);
    } else {
        let ca = dsp.accelerator.current_addr;
        dsp.accelerator.current_addr = (ca & !3) | (ca.wrapping_add(1) & 3);
    }

    if dsp.accelerator.current_addr.wrapping_sub(1) == dsp.accelerator.end_addr {
        dsp.accelerator.current_addr = dsp.accelerator.start_addr;
        // TODO: raise DSP exception
    }

    dsp.accelerator.set_current_addr(dsp.accelerator.current_addr);

    val
}

pub fn write_raw<const SYSTEM: SystemId>(dsp: &mut Dsp, mut ram: RamViewMut<'_>, value: u16) {
    if dsp.accelerator.current_addr & 0x8000_0000 != 0 {
        let off = dsp.accelerator.current_addr.wrapping_mul(2);
        write_source_byte::<SYSTEM>(dsp, &mut ram, off, (value >> 8) as u8);
        write_source_byte::<SYSTEM>(dsp, &mut ram, off.wrapping_add(1), value as u8);
        dsp.accelerator.current_addr = dsp.accelerator.current_addr.wrapping_add(1);
        // TODO: raise DSP exception
    } else {
        tracing::warn!(
            addr = format!("{:08X}", dsp.accelerator.current_addr),
            "Accelerator raw write blocked (high bit clear)"
        );
    }
}
