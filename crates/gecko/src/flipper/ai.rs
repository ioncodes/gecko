pub mod regs;

use crate::audio::{self, AidBlock};
use crate::flipper::dsp;
use crate::scheduler;
use crate::system::{System, SystemId};

const AUDIO_DMA_BLOCK_BYTES: u32 = 32;
const AUDIO_DMA_FRAMES_PER_BLOCK: u64 = 8;

pub struct AudioInterface {
    pub control: regs::AiControl,
    pub volume: regs::AiVolume,
    pub sample_counter: regs::AiSampleCounter,
    pub interrupt_timing: regs::AiInterruptTiming,
    pub sample_counter_base_cycle: u64,
    pub audio_dma_remaining_blocks: u16,
    pub audio_dma_current_addr: u32,
}

impl AudioInterface {
    pub fn new() -> Self {
        Self {
            control: regs::AiControl::from_raw(0).with_dsp_sample_rate(regs::SampleRate::Rate32KHz),
            volume: regs::AiVolume::from_raw(0),
            sample_counter: regs::AiSampleCounter::from_raw(0),
            interrupt_timing: regs::AiInterruptTiming::from_raw(0),
            sample_counter_base_cycle: 0,
            audio_dma_remaining_blocks: 0,
            audio_dma_current_addr: 0,
        }
    }

    #[inline(always)]
    pub fn interrupt_active(&self) -> bool {
        self.control.interrupt() && self.control.interrupt_mask()
    }

    /// Compute the current sample counter based on elapsed cycles
    #[inline(always)]
    pub fn sample_count(&self, system: SystemId, current_cycles: u64) -> u32 {
        if self.control.playback_status() != regs::Status::Play {
            return 0;
        }

        let elapsed = current_cycles.saturating_sub(self.sample_counter_base_cycle);
        let sample_rate = self.control.sample_rate().hz() as u128;
        ((elapsed as u128 * sample_rate) / scheduler::cpu_clock(system) as u128) as u32
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
pub fn start_audio_dma<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    if !sys.dsp.audio_dma_control.play() {
        return;
    }

    let blocks = sys.dsp.audio_dma_control.length();
    if blocks == 0 {
        stop_audio_dma(sys);
        return;
    }

    sys.scheduler.cancel(self::audio_dma_block_handler);
    sys.ai.audio_dma_remaining_blocks = blocks;
    sys.ai.audio_dma_current_addr = sys.dsp.audio_dma_start_addr.raw();
    sys.dsp.csr.set_dma_status(true);

    let addr = sys.dsp.audio_dma_start_addr.raw();
    let len = blocks as u32 * AUDIO_DMA_BLOCK_BYTES;

    tracing::debug!(addr = format!("{addr:08X}"), len, "AID DMA started");

    sys.scheduler
        .schedule_in(cycles_per_audio_dma_block(sys), self::audio_dma_block_handler);
}

#[inline(always)]
pub fn stop_audio_dma<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    sys.scheduler.cancel(self::audio_dma_block_handler);
    sys.ai.audio_dma_remaining_blocks = 0;
    sys.dsp.csr.set_dma_status(false);
}

#[inline(always)]
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn audio_dma_block_handler<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    if !sys.dsp.audio_dma_control.play() {
        stop_audio_dma(sys);
        return;
    }

    if sys.ai.audio_dma_remaining_blocks == 0 {
        sys.ai.audio_dma_remaining_blocks = sys.dsp.audio_dma_control.length();
        sys.ai.audio_dma_current_addr = sys.dsp.audio_dma_start_addr.raw();
    }

    if sys.ai.audio_dma_remaining_blocks == 0 {
        stop_audio_dma(sys);
        return;
    }

    let block_bytes: [u8; 32] = {
        let src = sys
            .mmio
            .virt_slice(sys.ai.audio_dma_current_addr, AUDIO_DMA_BLOCK_BYTES as usize);
        let mut buf = [0u8; 32];
        buf.copy_from_slice(src);
        buf
    };
    let frames: AidBlock = audio::decode_aid_block(&block_bytes);

    sys.audio_sink.push_stereo_block(&frames);
    sys.ai.audio_dma_current_addr = sys.ai.audio_dma_current_addr.wrapping_add(AUDIO_DMA_BLOCK_BYTES);

    sys.ai.audio_dma_remaining_blocks -= 1;

    if sys.ai.audio_dma_remaining_blocks == 0 {
        sys.dsp.csr.set_ai_interrupt(true);
        dsp::refresh_interrupts(sys);

        tracing::debug!(
            cycles = sys.scheduler.cycles,
            csr = format!("{:04X}", sys.dsp.csr.raw()),
            pi_intsr = format!("{:08X}", sys.pi.intsr.raw()),
            pi_intmr = format!("{:08X}", sys.pi.intmr.raw()),
            "Audio DMA completion"
        );
    }

    if sys.dsp.audio_dma_control.play() {
        sys.dsp.csr.set_dma_status(true);
        sys.scheduler
            .schedule_in(cycles_per_audio_dma_block(sys), self::audio_dma_block_handler);
    } else {
        stop_audio_dma(sys);
    }
}

#[inline(always)]
fn cycles_per_audio_dma_block<const SYSTEM: SystemId>(sys: &System<SYSTEM>) -> u64 {
    let sample_rate = sys.ai.control.aid_sample_rate_hz() as u64;
    AUDIO_DMA_FRAMES_PER_BLOCK * scheduler::cpu_clock(SYSTEM) / sample_rate
}

#[inline(always)]
pub fn refresh_interrupts<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    use crate::flipper::pi::InterruptFlag;

    let threshold = sys.ai.interrupt_timing.sample_count();
    if threshold != 0 {
        let count = sys.ai.sample_count(SYSTEM, sys.scheduler.cycles);
        sys.ai.sample_counter = regs::AiSampleCounter::from_raw(count);
        if count >= threshold {
            sys.ai.control = sys.ai.control.with_interrupt(true);
        }
    }

    if sys.ai.interrupt_active() {
        sys.pi.assert_interrupt(InterruptFlag::Ai);
    } else {
        sys.pi.clear_interrupt(InterruptFlag::Ai);
    }
}
