pub mod regs;

use crate::scheduler;
use crate::system::{System, SystemId};

const CLOCK_FREQUENCIES: [u64; 2] = [27_000_000, 54_000_000];

pub struct VideoInterface {
    pub vtr: regs::VerticalTiming,
    pub htr0: regs::HorizontalTiming0,
    pub htr1: regs::HorizontalTiming1,
    pub vto: regs::VerticalTimingOdd,
    pub vte: regs::VerticalTimingEven,
    pub dcr: regs::DisplayConfiguration,
    pub bbei: regs::BurstBlankingEvenInterval,
    pub bboi: regs::BurstBlankingOddInterval,
    pub tfbl: regs::TopFieldBase,
    pub tfbr: regs::TopFieldBaseRight,
    pub bfbl: regs::BottomFieldBase,
    pub bfbr: regs::BottomFieldBaseRight,
    pub dpv: regs::DisplayPositionVertical,
    pub dph: regs::DisplayPositionHorizontal,
    pub di0: regs::DisplayInterrupt0,
    pub di1: regs::DisplayInterrupt1,
    pub di2: regs::DisplayInterrupt2,
    pub di3: regs::DisplayInterrupt3,
    pub dl0: regs::DisplayLatch0,
    pub dl1: regs::DisplayLatch1,
    pub hsw: regs::HorizontalScalingWidth,
    pub hsr: regs::HorizontalScalingRegister,
    pub fct0: regs::FilterCoefficient0,
    pub fct1: regs::FilterCoefficient1,
    pub fct2: regs::FilterCoefficient2,
    pub fct3: regs::FilterCoefficient3,
    pub fct4: regs::FilterCoefficient4,
    pub fct5: regs::FilterCoefficient5,
    pub fct6: regs::FilterCoefficient6,
    pub viclk: regs::ViClockSelect,
    pub visel: regs::ViDtvStatus,
    pub unknown_70: regs::ViUnknown70,
    pub border_hbe: regs::BorderHbe,
    pub border_hbs: regs::BorderHbs,

    // VI timing state
    pub half_line_count: u32,
    pub ticks_last_line_start: u64,
    pub half_line_scheduled: bool,
}

impl VideoInterface {
    pub fn new() -> Self {
        VideoInterface {
            vtr: regs::VerticalTiming::from_raw(0),
            htr0: regs::HorizontalTiming0::from_raw(0),
            htr1: regs::HorizontalTiming1::from_raw(0),
            vto: regs::VerticalTimingOdd::from_raw(0),
            vte: regs::VerticalTimingEven::from_raw(0),
            dcr: regs::DisplayConfiguration::from_raw(0),
            bbei: regs::BurstBlankingEvenInterval::from_raw(0),
            bboi: regs::BurstBlankingOddInterval::from_raw(0),
            tfbl: regs::TopFieldBase::from_raw(0),
            tfbr: regs::TopFieldBaseRight::from_raw(0),
            bfbl: regs::BottomFieldBase::from_raw(0),
            bfbr: regs::BottomFieldBaseRight::from_raw(0),
            dpv: regs::DisplayPositionVertical::from_raw(0),
            dph: regs::DisplayPositionHorizontal::from_raw(0),
            di0: regs::DisplayInterrupt0::from_raw(0),
            di1: regs::DisplayInterrupt1::from_raw(0),
            di2: regs::DisplayInterrupt2::from_raw(0),
            di3: regs::DisplayInterrupt3::from_raw(0),
            dl0: regs::DisplayLatch0::from_raw(0),
            dl1: regs::DisplayLatch1::from_raw(0),
            hsw: regs::HorizontalScalingWidth::from_raw(0),
            hsr: regs::HorizontalScalingRegister::from_raw(0),
            fct0: regs::FilterCoefficient0::from_raw(0),
            fct1: regs::FilterCoefficient1::from_raw(0),
            fct2: regs::FilterCoefficient2::from_raw(0),
            fct3: regs::FilterCoefficient3::from_raw(0),
            fct4: regs::FilterCoefficient4::from_raw(0),
            fct5: regs::FilterCoefficient5::from_raw(0),
            fct6: regs::FilterCoefficient6::from_raw(0),
            viclk: regs::ViClockSelect::from_raw(0),
            visel: regs::ViDtvStatus::from_raw(0),
            unknown_70: regs::ViUnknown70::from_raw(0),
            border_hbe: regs::BorderHbe::from_raw(0),
            border_hbs: regs::BorderHbs::from_raw(0),
            half_line_count: 0,
            ticks_last_line_start: 0,
            half_line_scheduled: false,
        }
    }

    pub fn ticks_per_sample(&self, system: SystemId) -> u64 {
        let clock_idx = self.viclk.clock_select() as usize & 1;
        2 * scheduler::cpu_clock(system) / CLOCK_FREQUENCIES[clock_idx]
    }

    pub fn ticks_per_half_line(&self, system: SystemId) -> u64 {
        self.ticks_per_sample(system) * self.htr0.halfline_width() as u64
    }

    fn half_lines_per_even_field(&self) -> u32 {
        let equ = self.vtr.equalization_pulse() as u32;
        let acv = self.vtr.active_video() as u32;
        let pre_blank = self.vte.pre_blanking_in_half_lines() as u32;
        let post_blank = self.vte.post_blanking_in_half_lines() as u32;
        3 * equ + pre_blank + 2 * acv + post_blank
    }

    fn half_lines_per_odd_field(&self) -> u32 {
        let equ = self.vtr.equalization_pulse() as u32;
        let acv = self.vtr.active_video() as u32;
        let pre_blank = self.vto.pre_blanking_in_half_lines() as u32;
        let post_blank = self.vto.post_blanking_in_half_lines() as u32;
        3 * equ + pre_blank + 2 * acv + post_blank
    }

    fn total_half_lines(&self) -> u32 {
        self.half_lines_per_even_field() + self.half_lines_per_odd_field()
    }

    /// Returns true if any DI register has both IR_INT and IR_MASK set.
    #[inline(always)]
    pub fn vi_interrupt_active(&self) -> bool {
        (self.di0.interrupt() && self.di0.enable())
            || (self.di1.interrupt() && self.di1.enable())
            || (self.di2.interrupt() && self.di2.enable())
            || (self.di3.interrupt() && self.di3.enable())
    }

    /// Compute the current DPH value from the cycle count.
    pub fn dph_value(&self, system: SystemId, cycles: u64) -> u16 {
        let hl_width = self.htr0.halfline_width() as u64;
        let ticks_per_hl = self.ticks_per_half_line(system);
        if ticks_per_hl == 0 || hl_width == 0 {
            return 1;
        }

        let elapsed = cycles.saturating_sub(self.ticks_last_line_start);
        let raw = 1 + hl_width * elapsed / ticks_per_hl;
        raw.clamp(1, hl_width * 2) as u16
    }

    /// Called once per half-line from the scheduler event.
    /// Advances the half-line counter, updates DPV, and checks DI interrupts.
    pub fn on_half_line(&mut self, cycles: u64) {
        let total_hl = self.total_half_lines();
        if total_hl == 0 {
            return;
        }

        self.half_line_count += 1;
        if self.half_line_count >= total_hl {
            self.half_line_count = 0;
        }

        if (self.half_line_count & 1) == 0 {
            self.ticks_last_line_start = cycles;
        }

        self.dpv.set_vertical_count((1 + self.half_line_count / 2) as u16);

        let hl_width = self.htr0.halfline_width() as u32;
        let current_vct = 1 + self.half_line_count / 2;
        let current_parity = self.half_line_count & 1;

        macro_rules! check_di {
            ($di:expr) => {
                if $di.enable() {
                    let target_parity = if $di.horizontal_count() as u32 > hl_width {
                        1
                    } else {
                        0
                    };
                    if current_vct == $di.vertical_count() as u32 && current_parity == target_parity {
                        $di.set_interrupt(true);
                    }
                }
            };
        }

        check_di!(self.di0);
        check_di!(self.di1);
        check_di!(self.di2);
        check_di!(self.di3);
    }

    #[inline(always)]
    pub fn in_even_field(&self) -> bool {
        self.half_line_count >= self.half_lines_per_odd_field()
    }

    pub fn frame_dimensions(&self) -> (u32, u32) {
        let hsw_raw = self.hsw.raw();
        let width = (((hsw_raw >> 8) & 0x7F) * 16) as u32;
        let active_lines = self.vtr.active_video() as u32;
        let interlaced = self.dcr.interlaced();
        let height = if active_lines == 0 {
            480
        } else if interlaced {
            active_lines.saturating_mul(2)
        } else {
            active_lines
        };
        (if width == 0 { 640 } else { width }, height)
    }

    pub fn xfb_addr(&self) -> u32 {
        if self.dcr.interlaced() && self.in_even_field() {
            if self.bfbl.page_offset() {
                self.bfbl.xfb_addr() << 5
            } else {
                self.bfbl.xfb_addr()
            }
        } else {
            if self.tfbl.page_offset() {
                self.tfbl.xfb_addr() << 5
            } else {
                self.tfbl.xfb_addr()
            }
        }
    }
}

crate::mmio_device_dispatch! {
    read = vi_read,
    write = vi_write,
    registers = [
        regs::VerticalTiming,
        regs::HorizontalTiming0,
        regs::HorizontalTiming1,
        regs::VerticalTimingOdd,
        regs::VerticalTimingEven,
        regs::DisplayConfiguration,
        regs::BurstBlankingEvenInterval,
        regs::BurstBlankingOddInterval,
        regs::TopFieldBase,
        regs::TopFieldBaseRight,
        regs::BottomFieldBase,
        regs::BottomFieldBaseRight,
        regs::DisplayPositionVertical,
        regs::DisplayPositionHorizontal,
        regs::DisplayPositionCombined,
        regs::DisplayInterrupt0,
        regs::DisplayInterrupt1,
        regs::DisplayInterrupt2,
        regs::DisplayInterrupt3,
        regs::DisplayLatch0,
        regs::DisplayLatch1,
        regs::HorizontalScalingWidth,
        regs::HorizontalScalingRegister,
        regs::FilterCoefficient0,
        regs::FilterCoefficient1,
        regs::FilterCoefficient2,
        regs::FilterCoefficient3,
        regs::FilterCoefficient4,
        regs::FilterCoefficient5,
        regs::FilterCoefficient6,
        regs::ViClockSelect,
        regs::ViDtvStatus,
        regs::ViUnknown70,
        regs::BorderHbe,
        regs::BorderHbs,
    ],
}

#[inline(always)]
pub fn ensure_half_line_scheduled<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    if !sys.vi.half_line_scheduled {
        let ticks_per_hl = sys.vi.ticks_per_half_line(SYSTEM);
        if ticks_per_hl > 0 {
            sys.vi.half_line_scheduled = true;
            sys.scheduler.schedule_in(ticks_per_hl, |sys| {
                let prev_hl = sys.vi.half_line_count;
                sys.vi.on_half_line(sys.scheduler.cycles);
                sys.vi.half_line_scheduled = false;

                let curr_hl = sys.vi.half_line_count;
                let total_hl = sys.vi.total_half_lines();
                let odd_field_start = 0u32;
                let even_field_start = sys.vi.half_lines_per_odd_field();
                if total_hl > 0 && (curr_hl == odd_field_start || curr_hl == even_field_start) && curr_hl != prev_hl {
                    crate::flipper::gx::present_xfb(sys);
                }

                self::ensure_half_line_scheduled(sys);
                self::refresh_interrupts(sys);
            });
        }
    }
}

#[inline(always)]
pub fn refresh_interrupts<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    use crate::flipper::pi::InterruptFlag;

    if sys.vi.vi_interrupt_active() {
        sys.pi.assert_interrupt(InterruptFlag::Vi);
    } else {
        sys.pi.clear_interrupt(InterruptFlag::Vi);
    }
}

impl<const SYSTEM: SystemId> System<SYSTEM> {
    #[rustfmt::skip]
    pub fn render_xfb(&self) -> Vec<u32> {
        let video_format = self.vi.dcr.video_format();
        let width = video_format.columns();
        let height = video_format.lines();

        let mut pixels = vec![0u32; width * height];
        let xfb_addr = self.vi.xfb_addr();

        // XFB is YUY2 (YCbCr 4:2:2): each 32 bit word = [Y0][Cb][Y1][Cr] (big endian).
        // One word covers two adjacent pixels sharing Cb and Cr.
        let ycbcr_to_rgb = |y: f32, cb: f32, cr: f32| -> u32 {
            let r = (1.164 * y + 1.596 * cr).clamp(0.0, 255.0) as u8;
            let g = (1.164 * y - 0.813 * cr - 0.391 * cb).clamp(0.0, 255.0) as u8;
            let b = (1.164 * y + 2.018 * cb).clamp(0.0, 255.0) as u8;
            ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
        };

        for i in 0..(width * height / 2) {
            let word = self.mmio.phys_read_u32(xfb_addr + (i as u32) * 4);
            let y0 = ((word >> 24) & 0xFF) as f32 - 16.0;
            let cb = ((word >> 16) & 0xFF) as f32 - 128.0;
            let y1 = ((word >>  8) & 0xFF) as f32 - 16.0;
            let cr = ( word        & 0xFF) as f32 - 128.0;
            pixels[i * 2]     = ycbcr_to_rgb(y0, cb, cr);
            pixels[i * 2 + 1] = ycbcr_to_rgb(y1, cb, cr);
        }
        pixels
    }
}
