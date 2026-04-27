pub mod regs;

use crate::{
    flipper::dsp,
    system::{System, SystemId},
};

const CPU_CORE_CLOCK: u64 = 486_000_000;
const AUDIO_DMA_BLOCK_BYTES: u32 = 32;
const AUDIO_DMA_FRAMES_PER_BLOCK: u64 = 8;

pub struct AudioInterface {
    pub control: regs::AiControl,
    pub volume: regs::AiVolume,
    pub sample_counter: regs::AiSampleCounter,
    pub interrupt_timing: regs::AiInterruptTiming,
    pub sample_counter_base_cycle: u64,
    pub audio_dma_remaining_blocks: u16,
}

impl AudioInterface {
    pub fn new() -> Self {
        Self {
            control: regs::AiControl::from_raw(0),
            volume: regs::AiVolume::from_raw(0),
            sample_counter: regs::AiSampleCounter::from_raw(0),
            interrupt_timing: regs::AiInterruptTiming::from_raw(0),
            sample_counter_base_cycle: 0,
            audio_dma_remaining_blocks: 0,
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
        let sample_rate = match self.control.sample_rate() {
            regs::SampleRate::Rate32KHz => 32_000,
            regs::SampleRate::Rate48KHz => 48_000,
        };
        ((elapsed as u128 * sample_rate) / CPU_CORE_CLOCK as u128) as u32
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
pub fn start_audio_dma<const SYSTEM: SystemId>(gc: &mut System<SYSTEM>) {
    if !gc.dsp.audio_dma_control.play() {
        return;
    }

    let blocks = gc.dsp.audio_dma_control.length();
    if blocks == 0 {
        stop_audio_dma(gc);
        return;
    }

    gc.scheduler.cancel(self::audio_dma_block_handler);
    gc.ai.audio_dma_remaining_blocks = blocks;
    gc.dsp.csr.set_dma_status(true);

    let addr = gc.dsp.audio_dma_start_addr.raw();
    let len = blocks as u32 * AUDIO_DMA_BLOCK_BYTES;

    tracing::debug!(addr = format!("{addr:08X}"), len, "Audio DMA");

    // TODO: actually stream samples to audio output.
    gc.scheduler
        .schedule_in(cycles_per_audio_dma_block(gc), self::audio_dma_block_handler);
}

#[inline(always)]
pub fn stop_audio_dma<const SYSTEM: SystemId>(gc: &mut System<SYSTEM>) {
    gc.scheduler.cancel(self::audio_dma_block_handler);
    gc.ai.audio_dma_remaining_blocks = 0;
    gc.dsp.csr.set_dma_status(false);
}

#[inline(always)]
pub fn audio_dma_block_handler<const SYSTEM: SystemId>(gc: &mut System<SYSTEM>) {
    if !gc.dsp.audio_dma_control.play() {
        stop_audio_dma(gc);
        return;
    }

    if gc.ai.audio_dma_remaining_blocks == 0 {
        gc.ai.audio_dma_remaining_blocks = gc.dsp.audio_dma_control.length();
    }

    if gc.ai.audio_dma_remaining_blocks == 0 {
        stop_audio_dma(gc);
        return;
    }

    gc.ai.audio_dma_remaining_blocks -= 1;

    if gc.ai.audio_dma_remaining_blocks == 0 {
        gc.dsp.csr.set_ai_interrupt(true);
        dsp::refresh_interrupts(gc);

        tracing::debug!(
            cycles = gc.scheduler.cycles,
            csr = format!("{:04X}", gc.dsp.csr.raw()),
            pi_intsr = format!("{:08X}", gc.pi.intsr.raw()),
            pi_intmr = format!("{:08X}", gc.pi.intmr.raw()),
            "Audio DMA completion"
        );

        gc.ai.audio_dma_remaining_blocks = gc.dsp.audio_dma_control.length();
    }

    if gc.dsp.audio_dma_control.play() && gc.ai.audio_dma_remaining_blocks != 0 {
        gc.dsp.csr.set_dma_status(true);
        gc.scheduler
            .schedule_in(cycles_per_audio_dma_block(gc), self::audio_dma_block_handler);
    } else {
        stop_audio_dma(gc);
    }
}

#[inline(always)]
fn cycles_per_audio_dma_block<const SYSTEM: SystemId>(gc: &System<SYSTEM>) -> u64 {
    let sample_rate = if gc.ai.control.dsp_sample_rate() {
        32_000
    } else {
        48_000
    };

    AUDIO_DMA_FRAMES_PER_BLOCK * CPU_CORE_CLOCK / sample_rate
}

#[inline(always)]
pub fn refresh_interrupts<const SYSTEM: SystemId>(gc: &mut System<SYSTEM>) {
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
