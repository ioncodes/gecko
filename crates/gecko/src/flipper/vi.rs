pub mod regs;

use crate::{
    flipper::pi::InterruptFlag,
    gamecube::GameCube,
    mmio::{
        constants::VI_BASE,
        traits::{MmioAccess, MmioRegister, MmioRw},
    },
    scheduler::EventKind,
};

const CPU_CORE_CLOCK: u64 = 486_000_000;
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

    pub fn ticks_per_sample(&self) -> u64 {
        let clock_idx = self.viclk.clock_select() as usize & 1;
        2 * CPU_CORE_CLOCK / CLOCK_FREQUENCIES[clock_idx]
    }

    pub fn ticks_per_half_line(&self) -> u64 {
        self.ticks_per_sample() * self.htr0.halfline_width() as u64
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
    pub fn vi_interrupt_active(&self) -> bool {
        (self.di0.interrupt() && self.di0.enable())
            || (self.di1.interrupt() && self.di1.enable())
            || (self.di2.interrupt() && self.di2.enable())
            || (self.di3.interrupt() && self.di3.enable())
    }

    /// Compute the current DPH value from the cycle count.
    pub fn dph_value(&self, cycles: u64) -> u16 {
        let hl_width = self.htr0.halfline_width() as u64;
        let ticks_per_hl = self.ticks_per_half_line();
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

    pub fn xfb_addr(&self) -> u32 {
        if self.tfbl.page_offset() {
            self.tfbl.xfb_addr() << 5
        } else {
            self.tfbl.xfb_addr()
        }
    }
}

impl MmioRw for VideoInterface {
    const BASE: u32 = VI_BASE;
    const NAME: &'static str = "VI";

    crate::impl_mmio_dispatch!(
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
    );
}

impl GameCube {
    pub fn maybe_schedule_vi_half_line(&mut self) {
        if !self.vi.half_line_scheduled {
            let ticks_per_hl = self.vi.ticks_per_half_line();
            if ticks_per_hl > 0 {
                self.vi.half_line_scheduled = true;
                self.scheduler.schedule_in(ticks_per_hl, EventKind::ViHalfLine);
            }
        }
    }

    pub fn check_vi_interrupts(&mut self) {
        if self.vi.vi_interrupt_active() {
            self.pi.assert_interrupt(InterruptFlag::Vi);
        } else {
            self.pi.clear_interrupt(InterruptFlag::Vi);
        }
    }

    #[rustfmt::skip]
    pub fn render_xfb(&self) -> Vec<u32> {
        let video_format = self.vi.dcr.video_format();
        let width = video_format.columns();
        let height = video_format.lines();

        let mut pixels = vec![0u32; width * height];
        let xfb_addr = self.vi.xfb_addr();

        // XFB is YUY2 (YCbCr 4:2:2): each 32-bit word = [Y0][Cb][Y1][Cr] (big-endian)
        // One word -> two adjacent pixels sharing Cb and Cr.
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
