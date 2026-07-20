use std::collections::VecDeque;

use crate::flipper::vi::regs::RefreshRate;
use crate::system::{self, System, SystemId};

pub const TIMEBASE_DIVISOR: u64 = 12;
pub const DSP_BATCH_SIZE: u64 = 1024;

#[inline(always)]
#[rustfmt::skip]
pub const fn cpu_clock(system: SystemId) -> u64 {
    match system {
        system::WII => 729_000_000,
        system::GC  => 486_000_000,
        _ => unreachable!(),
    }
}

#[inline(always)]
#[rustfmt::skip]
pub const fn cpu_cycles_per_dsp_tick(system: SystemId) -> u64 {
    match system {
        system::WII => 9, // 729 MHz / 81 MHz
        system::GC  => 6, // 486 MHz / 81 MHz
        _ => unreachable!(),
    }
}

#[inline(always)]
pub const fn microseconds_to_cycles(system: SystemId, us: u64) -> u64 {
    us * (self::cpu_clock(system) / 1_000_000)
}

pub type ScheduledFn<const SYSTEM: SystemId> = fn(&mut System<SYSTEM>);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u16)]
pub enum Handler {
    Vsync = 1,
    InputPoll = 2,
    DspBatch = 3,
    DecUnderflow = 4,
    CpPump = 5,
    AiAudioDmaBlock = 6,
    DiTransferDone = 7,
    ViHalfLine = 8,
    AramDma = 9,
    IpcDeliver = 10,
    MemcardCmdDone0 = 11,
    MemcardCmdDone1 = 12,
    MemcardCmdDone2 = 13,
    Fps = 14,
}

impl Handler {
    pub const fn memcard_cmd_done(channel: usize) -> Self {
        match channel {
            0 => Handler::MemcardCmdDone0,
            1 => Handler::MemcardCmdDone1,
            2 => Handler::MemcardCmdDone2,
            _ => unreachable!(),
        }
    }

    const ALL: [Handler; 14] = [
        Handler::Vsync,
        Handler::InputPoll,
        Handler::DspBatch,
        Handler::DecUnderflow,
        Handler::CpPump,
        Handler::AiAudioDmaBlock,
        Handler::DiTransferDone,
        Handler::ViHalfLine,
        Handler::AramDma,
        Handler::IpcDeliver,
        Handler::MemcardCmdDone0,
        Handler::MemcardCmdDone1,
        Handler::MemcardCmdDone2,
        Handler::Fps,
    ];

    pub fn from_u16(id: u16) -> Option<Self> {
        Self::ALL.get(id.checked_sub(1)? as usize).copied()
    }

    #[inline(always)]
    pub fn resolve<const SYSTEM: SystemId>(self) -> ScheduledFn<SYSTEM> {
        match self {
            Handler::Vsync => self::vsync_handler::<SYSTEM>,
            Handler::InputPoll => self::input_poll_handler::<SYSTEM>,
            Handler::DspBatch => self::dsp_batch_handler::<SYSTEM>,
            Handler::DecUnderflow => crate::gekko::dec::underflow_handler::<SYSTEM>,
            Handler::CpPump => crate::flipper::cp::pump_handler::<SYSTEM>,
            Handler::AiAudioDmaBlock => crate::flipper::ai::audio_dma_block_handler::<SYSTEM>,
            Handler::DiTransferDone => crate::dvd::transfer_done_handler::<SYSTEM>,
            Handler::ViHalfLine => crate::flipper::vi::half_line_handler::<SYSTEM>,
            Handler::AramDma => crate::flipper::dsp::regs::aram_dma_handler::<SYSTEM>,
            Handler::IpcDeliver => crate::starlet::deliver_pending::<SYSTEM>,
            Handler::MemcardCmdDone0 => crate::flipper::exi::memcard_cmd_done::<0, SYSTEM>,
            Handler::MemcardCmdDone1 => crate::flipper::exi::memcard_cmd_done::<1, SYSTEM>,
            Handler::MemcardCmdDone2 => crate::flipper::exi::memcard_cmd_done::<2, SYSTEM>,
            #[cfg(feature = "fps-counter")]
            Handler::Fps => crate::fps::fps_handler::<SYSTEM>,
            #[cfg(not(feature = "fps-counter"))]
            Handler::Fps => |_| {},
        }
    }
}

#[derive(Clone, Copy)]
struct ScheduledEvent {
    deadline: u64,
    handler: Handler,
}

pub struct Scheduler<const SYSTEM: SystemId> {
    pub cycles: u64,
    pub(crate) next_deadline: u64,
    pub(crate) timebase_offset: i64,
    events: VecDeque<ScheduledEvent>,
    #[cfg(feature = "jit-stats")]
    pub(crate) event_fire_counts: rustc_hash::FxHashMap<Handler, u64>,
}

impl<const SYSTEM: SystemId> Scheduler<SYSTEM> {
    pub fn empty() -> Self {
        Scheduler {
            cycles: 0,
            next_deadline: u64::MAX,
            timebase_offset: 0,
            events: VecDeque::with_capacity(8),
            #[cfg(feature = "jit-stats")]
            event_fire_counts: rustc_hash::FxHashMap::default(),
        }
    }

    #[inline(always)]
    pub fn refresh_deadline(&mut self) {
        self.next_deadline = self.events.front().map_or(u64::MAX, |e| e.deadline);
    }

    pub fn timebase(&self) -> u64 {
        ((self.cycles / TIMEBASE_DIVISOR) as i64 + self.timebase_offset) as u64
    }

    pub fn set_timebase_lower(&mut self, val: u32) {
        let current = self.timebase();
        let new_tb = (current & 0xFFFF_FFFF_0000_0000) | val as u64;
        self.timebase_offset = new_tb as i64 - (self.cycles / TIMEBASE_DIVISOR) as i64;
    }

    pub fn set_timebase_upper(&mut self, val: u32) {
        let current = self.timebase();
        let new_tb = ((val as u64) << 32) | (current & 0xFFFF_FFFF);
        self.timebase_offset = new_tb as i64 - (self.cycles / TIMEBASE_DIVISOR) as i64;
    }

    pub fn timebase_lower(&self) -> u32 {
        self.timebase() as u32
    }

    pub fn timebase_upper(&self) -> u32 {
        (self.timebase() >> 32) as u32
    }

    /// Insert an event keeping the deque sorted by deadline (earliest first).
    pub fn schedule_at(&mut self, deadline: u64, handler: Handler) {
        let pos = self.events.partition_point(|e| e.deadline <= deadline);
        self.events.insert(pos, ScheduledEvent { deadline, handler });
        self.next_deadline = self.next_deadline.min(deadline);
    }

    pub fn cancel(&mut self, handler: Handler) {
        self.events.retain(|e| e.handler != handler);
        self.refresh_deadline();
    }

    pub fn schedule_in(&mut self, delay: u64, handler: Handler) {
        let deadline = self.cycles + delay;
        self.schedule_at(deadline, handler);
    }

    #[inline(always)]
    pub fn poll(&mut self) -> Option<Handler> {
        let front = self.events.front()?;
        if self.cycles >= front.deadline {
            let handler = self.events.pop_front().unwrap().handler;
            self.refresh_deadline();
            #[cfg(feature = "jit-stats")]
            {
                *self.event_fire_counts.entry(handler).or_insert(0) += 1;
            }
            Some(handler)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn next_deadline(&self) -> u64 {
        self.next_deadline
    }

    pub fn save_state(&self, w: &mut crate::savestate::StateWriter) {
        w.u64(self.cycles);
        w.i64(self.timebase_offset);

        w.u32(self.events.len() as u32);
        for event in &self.events {
            w.u16(event.handler as u16);
            w.u64(event.deadline);
        }
    }

    pub fn load_state(
        &mut self,
        r: &mut crate::savestate::StateReader<'_>,
    ) -> Result<(), crate::savestate::StateError> {
        self.cycles = r.u64()?;
        self.timebase_offset = r.i64()?;

        self.events.clear();
        let count = r.u32()?;
        for _ in 0..count {
            let id = r.u16()?;
            let deadline = r.u64()?;
            let handler =
                Handler::from_u16(id).ok_or(crate::savestate::StateError::Corrupt("unknown scheduler handler id"))?;
            self.events.push_back(ScheduledEvent { deadline, handler });
        }

        self.refresh_deadline();
        Ok(())
    }
}

impl Scheduler<{ crate::system::GC }> {
    pub fn new_gamecube() -> Self {
        Self::with_default_events()
    }
}

impl Scheduler<{ crate::system::WII }> {
    pub fn new_wii() -> Self {
        Self::with_default_events()
    }
}

impl<const SYSTEM: SystemId> Scheduler<SYSTEM> {
    fn with_default_events() -> Self {
        let mut s = Self::empty();
        let initial_refresh_rate = RefreshRate::Hz60;
        s.schedule_at(initial_refresh_rate.cycles_per_frame(SYSTEM), Handler::Vsync);
        s.schedule_at(
            crate::gekko::dec::cycles_until_underflow(u32::MAX),
            Handler::DecUnderflow,
        );
        s.schedule_at(crate::flipper::cp::PUMP_INTERVAL_CYCLES, Handler::CpPump);
        s.schedule_at(self::input_poll_interval(SYSTEM), Handler::InputPoll);
        #[cfg(feature = "fps-counter")]
        s.schedule_at(self::cpu_clock(SYSTEM), Handler::Fps);
        s
    }
}

/// Reschedules itself every frame.
pub fn vsync_handler<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    if !sys.vi_present_seen_this_frame {
        sys.vsync_pending = true;

        #[cfg(feature = "fps-counter")]
        {
            sys.fps_counter.vsync_count += 1;
        }
    }

    sys.vi_present_seen_this_frame = false;
    let rate = sys.vi.dcr.video_format().refresh_rate();
    sys.scheduler.schedule_in(rate.cycles_per_frame(SYSTEM), Handler::Vsync);
}

pub fn reseed_vsync<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    let rate = sys.vi.dcr.video_format().refresh_rate();
    sys.scheduler.cancel(Handler::Vsync);
    sys.scheduler.schedule_in(rate.cycles_per_frame(SYSTEM), Handler::Vsync);
}

#[inline(always)]
pub const fn input_poll_interval(system: SystemId) -> u64 {
    self::cpu_clock(system) / crate::hollywood::ipc::usb::REPORT_HZ
}

/// Reschedules itself at the Wiimote's 200Hz (TODO VERIFY) report rate. Samples
/// fresh host input on both systems and, on Wii, emits one HID input report
/// per tick (continuous mode) or only on change (non continuous).
pub fn input_poll_handler<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    sys.sample_host_input();

    if SYSTEM == system::WII {
        sys.starlet.tick_wiimote();
    }

    sys.scheduler
        .schedule_in(self::input_poll_interval(SYSTEM), Handler::InputPoll);
}

#[inline(always)]
pub const fn dsp_batch_interval(system: SystemId) -> u64 {
    self::cpu_cycles_per_dsp_tick(system) * self::DSP_BATCH_SIZE
}

pub fn dsp_batch_handler<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    sys.execute_dsp_batch();
    if sys.dsp.csr.halt() || sys.dsp.csr.reset() {
        return;
    }

    let in_idle_wait = sys.dsp.parked_in_mailbox_wait();
    let pending_interrupt = sys.dsp.csr.pi_interrupt() && sys.dsp.registers.status.external_interrupt_enable();

    if in_idle_wait && !pending_interrupt {
        sys.dsp.scheduler_suspended = true;
        #[cfg(feature = "jit-stats")]
        crate::flipper::dsp::DSP_SUSPEND_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return;
    }

    sys.scheduler
        .schedule_in(self::dsp_batch_interval(SYSTEM), Handler::DspBatch);
}
