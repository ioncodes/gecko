use crate::flipper::ai;
use crate::mmio::traits::{MmioAccess, WriteMask};
use crate::system::{System, SystemId};
use chapa::BitEnum;

// 0xCC006C00  4  R/W  AICR - Audio Interface Control Register

#[derive(BitEnum, Debug, PartialEq, Eq)]
pub enum Status {
    StopOrPause = 0,
    Play = 1,
}

#[derive(BitEnum, Debug, PartialEq, Eq)]
pub enum SampleRate {
    Rate48KHz = 0,
    Rate32KHz = 1,
}

impl SampleRate {
    #[inline(always)]
    pub fn hz(self) -> u32 {
        match self {
            SampleRate::Rate48KHz => 48_000,
            SampleRate::Rate32KHz => 32_000,
        }
    }
}

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AiControl {
    #[bits(0, alias = "pstat")]
    pub playback_status: Status,

    #[bits(1, alias = "afr")]
    pub sample_rate: SampleRate,

    #[bits(2, alias = "aiintmsk")]
    pub interrupt_mask: bool,

    #[bits(3, alias = "aiint")]
    pub interrupt: bool,

    #[bits(4, alias = "aiintvld")]
    pub interrupt_valid: bool,

    #[bits(5, alias = "screset")]
    pub sample_counter_reset: bool,

    #[bits(6)]
    pub dsp_sample_rate: SampleRate,
}
crate::mmio_reg!(AiControl: u32 @ 0xCC006C00);

impl AiControl {
    /// AID DMA streaming rate in Hz (driven by the DSP sample rate field).
    #[inline(always)]
    pub fn aid_sample_rate_hz(self) -> u32 {
        self.dsp_sample_rate().hz()
    }
}

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for AiControl {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.ai.control
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        let mut cr = sys.ai.control;
        let old_aid_sample_rate = cr.aid_sample_rate_hz();

        // AIINT is w1c
        if self.interrupt() {
            cr = cr.with_interrupt(false);
        }

        // Passthrough
        cr = cr
            .with_playback_status(self.playback_status())
            .with_sample_rate(self.sample_rate())
            .with_interrupt_mask(self.interrupt_mask())
            .with_interrupt_valid(self.interrupt_valid())
            .with_dsp_sample_rate(self.dsp_sample_rate());

        // SCRESET resets the sample counter
        if self.sample_counter_reset() {
            sys.ai.sample_counter_base_cycle = sys.scheduler.cycles;
            sys.ai.sample_counter = AiSampleCounter::from_raw(0);
        }

        sys.ai.control = cr;

        let new_aid_sample_rate = sys.ai.control.aid_sample_rate_hz();
        if old_aid_sample_rate != new_aid_sample_rate {
            tracing::debug!(
                old_sample_rate = old_aid_sample_rate,
                new_sample_rate = new_aid_sample_rate,
                "AID sample rate changed"
            );
            sys.audio_sink.set_sample_rate(new_aid_sample_rate);
        }

        ai::refresh_interrupts(sys);
    }
}

// 0xCC006C04  4  R/W  AIVR - Audio Interface Volume Register
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AiVolume {
    #[bits(0..=7)]
    pub left: u8,

    #[bits(8..=15)]
    pub right: u8,
}
crate::mmio_reg!(AiVolume: u32 @ 0xCC006C04);
crate::mmio_default_access!(AiVolume => System.ai.volume);

// 0xCC006C08  4  R  AISCNT - Audio Interface Sample Counter

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AiSampleCounter {
    #[bits(0..=30)]
    pub sample_count: u32,
}
crate::mmio_reg!(AiSampleCounter: u32 @ 0xCC006C08);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for AiSampleCounter {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        let count = sys.ai.sample_count(SYSTEM, sys.scheduler.cycles);
        AiSampleCounter::from_raw(count)
    }

    fn write(self, _gc: &mut System<SYSTEM>, _: WriteMask) {
        tracing::warn!("attempted to write to read-only AiSampleCounter");
    }
}

// 0xCC006C0C  4  R/W  AIIT - Audio Interface Interrupt Timing
#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct AiInterruptTiming {
    #[bits(0..=30)]
    pub sample_count: u32,
}
crate::mmio_reg!(AiInterruptTiming: u32 @ 0xCC006C0C);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for AiInterruptTiming {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.ai.interrupt_timing
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.ai.interrupt_timing = self;
        ai::refresh_interrupts(sys);
    }
}
