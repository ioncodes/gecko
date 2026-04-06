use super::AudioInterface;
use crate::mmio::traits::MmioAccess;
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

crate::mmio_register! {
    AiControl: u32 @ 0xCC006C00 {
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
        pub dsp_sample_rate: bool,
    }
}

impl MmioAccess<AudioInterface> for AiControl {
    fn read(ai: &AudioInterface) -> Self {
        ai.control
    }

    fn write(self, ai: &mut AudioInterface) {
        let mut cr = ai.control;

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
            ai.sample_counter_reset_pending = true;
            ai.sample_counter = AiSampleCounter::from_raw(0);
        }

        ai.control = cr;
    }
}

// 0xCC006C04  4  R/W  AIVR - Audio Interface Volume Register
crate::mmio_register! {
    AiVolume: u32 @ 0xCC006C04 => AudioInterface.volume {
        #[bits(0..=7)]
        pub left: u8,

        #[bits(8..=15)]
        pub right: u8,
    }
}

// 0xCC006C08  4  R  AISCNT - Audio Interface Sample Counter

crate::mmio_register! {
    AiSampleCounter: u32 @ 0xCC006C08 {
        #[bits(0..=30)]
        pub sample_count: u32,
    }
}

impl MmioAccess<AudioInterface> for AiSampleCounter {
    fn read(ai: &AudioInterface) -> Self {
        ai.sample_counter
    }

    fn write(self, _ai: &mut AudioInterface) {
        tracing::warn!("attempted to write to read-only AiSampleCounter");
    }
}

// 0xCC006C0C  4  R/W  AIIT - Audio Interface Interrupt Timing
crate::mmio_register! {
    AiInterruptTiming: u32 @ 0xCC006C0C => AudioInterface.interrupt_timing {
        #[bits(0..=30)]
        pub sample_count: u32,
    }
}
