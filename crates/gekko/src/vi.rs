#[cfg(test)]
mod tests;
pub mod regs;

use crate::{
    gekko::Gekko,
    mmio::constants::VI_BASE,
    mmio::traits::{MmioAccess, MmioRegister},
};

pub struct Vi {
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
    pub border_hbe: regs::BorderHbe,
    pub border_hbs: regs::BorderHbs,
}

impl Vi {
    pub fn new() -> Self {
        Vi {
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
            border_hbe: regs::BorderHbe::from_raw(0),
            border_hbs: regs::BorderHbs::from_raw(0),
        }
    }

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
        regs::BorderHbe,
        regs::BorderHbs,
    );

    pub fn xfb_addr(&self) -> u32 {
        (self.tfbl.xfb_addr() << 9) | ((self.tfbl.page_offset() as u32) << 24)
    }

    pub fn mmio_read_u8(&self, offset: u32) -> u8 {
        self.read_raw(VI_BASE + offset, 1).unwrap_or_else(|| {
            tracing::error!(offset = format!("{offset:08X}"), "unhandled VI read_u8");
            0
        }) as u8
    }

    pub fn mmio_write_u8(&mut self, offset: u32, val: u8) {
        if !self.write_raw(VI_BASE + offset, 1, val as u32) {
            tracing::error!(offset = format!("{offset:#08X}"), "unhandled VI write_u8");
        }
    }

    pub fn mmio_read_u16(&self, offset: u32) -> u16 {
        self.read_raw(VI_BASE + offset, 2).unwrap_or_else(|| {
            tracing::error!(offset = format!("{offset:#08X}"), "unhandled VI read_u16");
            0
        }) as u16
    }

    pub fn mmio_write_u16(&mut self, offset: u32, val: u16) {
        if !self.write_raw(VI_BASE + offset, 2, val as u32) {
            tracing::error!(offset = format!("{offset:#08X}"), "unhandled VI write_u16");
        }
    }

    pub fn mmio_read_u32(&self, offset: u32) -> u32 {
        self.read_raw(VI_BASE + offset, 4).unwrap_or_else(|| {
            tracing::error!(offset = format!("{offset:#08X}"), "unhandled VI read_u32");
            0
        })
    }

    pub fn mmio_write_u32(&mut self, offset: u32, val: u32) {
        if !self.write_raw(VI_BASE + offset, 4, val) {
            tracing::error!(offset = format!("{offset:#08X}"), "unhandled VI write_u32");
        }
    }
}

impl Gekko {
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
