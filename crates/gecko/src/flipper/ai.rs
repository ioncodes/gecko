pub mod regs;

use crate::{flipper::dsp, gamecube::GameCube};

/// CPU cycles per AI sample at 32kHz: 486MHz / 32000 = 15187
const CYCLES_PER_SAMPLE_32K: u64 = 15187;
/// CPU cycles per AI sample at 48kHz: 486MHz / 48000 = 10125
const CYCLES_PER_SAMPLE_48K: u64 = 10125;

pub struct AudioInterface {
    pub control: regs::AiControl,
    pub volume: regs::AiVolume,
    pub sample_counter: regs::AiSampleCounter,
    pub interrupt_timing: regs::AiInterruptTiming,
    pub sample_counter_base_cycle: u64,
}

impl AudioInterface {
    pub fn new() -> Self {
        Self {
            control: regs::AiControl::from_raw(0),
            volume: regs::AiVolume::from_raw(0),
            sample_counter: regs::AiSampleCounter::from_raw(0),
            interrupt_timing: regs::AiInterruptTiming::from_raw(0),
            sample_counter_base_cycle: 0,
        }
    }

    #[inline(always)]
    pub fn interrupt_active(&self) -> bool {
        self.control.interrupt() && self.control.interrupt_mask()
    }

    /// Compute the current sample counter based on elapsed cycles
    #[inline(always)]
    pub fn sample_count(&self, current_cycles: u64) -> u32 {
        if self.control.playback_status() != regs::Status::Play {
            return 0;
        }

        let elapsed = current_cycles.saturating_sub(self.sample_counter_base_cycle);
        let cycles_per_sample = match self.control.sample_rate() {
            regs::SampleRate::Rate32KHz => CYCLES_PER_SAMPLE_32K,
            regs::SampleRate::Rate48KHz => CYCLES_PER_SAMPLE_48K,
        };
        (elapsed / cycles_per_sample) as u32
    }
}

crate::mmio_device_dispatch! {
    read = ai_read,
    write = ai_write,
    registers = [
        regs::AiControl,
        regs::AiVolume,
        regs::AiInterruptTiming,
        regs::AiSampleCounter,
    ],
}

#[inline(always)]
pub fn start_audio_dma(gc: &mut GameCube) {
    if !gc.dsp.audio_dma_control.play() {
        return;
    }

    let addr = gc.dsp.audio_dma_start_addr.raw();
    let len = gc.dsp.audio_dma_control.length() as u32 * 32;

    tracing::warn!(addr = format!("{addr:08X}"), len, "Audio DMA");

    // TODO: actually stream samples to audio output.
    const AUDIO_DMA_DELAY: u64 = 10_000;
    gc.scheduler.schedule_in(AUDIO_DMA_DELAY, |gc| {
        gc.dsp.audio_dma_control.set_length(0);
        gc.dsp.csr.set_dma_status(false);
        gc.dsp.csr.set_ai_interrupt(true);
        // gc.dsp.audio_dma_control.set_play(false);
        dsp::refresh_interrupts(gc);
    });
}

#[inline(always)]
pub fn refresh_interrupts(gc: &mut GameCube) {
    use crate::flipper::pi::InterruptFlag;

    let threshold = gc.ai.interrupt_timing.sample_count();
    if threshold != 0 {
        let count = gc.ai.sample_count(gc.scheduler.cycles);
        gc.ai.sample_counter = regs::AiSampleCounter::from_raw(count);
        if count >= threshold {
            gc.ai.control = gc.ai.control.with_interrupt(true);
        }
    }

    if gc.ai.interrupt_active() {
        gc.pi.assert_interrupt(InterruptFlag::Ai);
    } else {
        gc.pi.clear_interrupt(InterruptFlag::Ai);
    }
}
