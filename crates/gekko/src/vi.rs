#[cfg(test)]
mod tests;

pub mod regs;

use crate::{
    gekko::Gekko,
    mmio::constants::VI_BASE,
    mmio::traits::{MmioAccess, MmioRegister},
};

pub struct Vi {
    pub dcr: regs::DisplayConfiguration,
    pub top_field_base: regs::TopFieldBase,
    pub bottom_field_base: regs::BottomFieldBase,
}

impl Vi {
    pub fn new() -> Self {
        Vi {
            dcr: regs::DisplayConfiguration::from_raw(0),
            top_field_base: regs::TopFieldBase::from_raw(0),
            bottom_field_base: regs::BottomFieldBase::from_raw(0),
        }
    }

    crate::impl_mmio_dispatch!(
        regs::DisplayConfiguration,
        regs::TopFieldBase,
        regs::BottomFieldBase,
    );

    pub fn xfb_addr(&self) -> u32 {
        let top = self.top_field_base;
        (top.xfb_addr() << 9) | ((top.page_offset() as u32) << 24)
    }

    pub fn mmio_read_u8(&self, offset: u32) -> u8 {
        self.read_raw(VI_BASE + offset, 1).unwrap_or_else(|| {
            tracing::warn!(offset = format!("{offset:08X}"), "unhandled VI read_u8");
            0
        }) as u8
    }

    pub fn mmio_write_u8(&mut self, offset: u32, val: u8) {
        if !self.write_raw(VI_BASE + offset, 1, val as u32) {
            tracing::warn!(offset = format!("{offset:#08X}"), "unhandled VI write_u8");
        }
    }

    pub fn mmio_read_u16(&self, offset: u32) -> u16 {
        self.read_raw(VI_BASE + offset, 2).unwrap_or_else(|| {
            tracing::warn!(offset = format!("{offset:#08X}"), "unhandled VI read_u16");
            0
        }) as u16
    }

    pub fn mmio_write_u16(&mut self, offset: u32, val: u16) {
        if !self.write_raw(VI_BASE + offset, 2, val as u32) {
            tracing::warn!(offset = format!("{offset:#08X}"), "unhandled VI write_u16");
        }
    }

    pub fn mmio_read_u32(&self, offset: u32) -> u32 {
        self.read_raw(VI_BASE + offset, 4).unwrap_or_else(|| {
            tracing::warn!(offset = format!("{offset:#08X}"), "unhandled VI read_u32");
            0
        })
    }

    pub fn mmio_write_u32(&mut self, offset: u32, val: u32) {
        if !self.write_raw(VI_BASE + offset, 4, val) {
            tracing::warn!(offset = format!("{offset:#08X}"), "unhandled VI write_u32");
        }
    }
}

pub const XFB_WIDTH: usize = 640;
pub const XFB_HEIGHT: usize = 574;

impl Gekko {
    #[rustfmt::skip]
    pub fn render_xfb(&self) -> Vec<u32> {
        let mut pixels = vec![0u32; XFB_WIDTH * XFB_HEIGHT];
        let xfb_addr = self.vi.xfb_addr();

        // XFB is YUY2 (YCbCr 4:2:2): each 32-bit word = [Y0][Cb][Y1][Cr] (big-endian)
        // One word -> two adjacent pixels sharing Cb and Cr.
        let ycbcr_to_rgb = |y: f32, cb: f32, cr: f32| -> u32 {
            let r = (1.164 * y + 1.596 * cr).clamp(0.0, 255.0) as u8;
            let g = (1.164 * y - 0.813 * cr - 0.391 * cb).clamp(0.0, 255.0) as u8;
            let b = (1.164 * y + 2.018 * cb).clamp(0.0, 255.0) as u8;
            ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
        };

        for i in 0..(XFB_WIDTH * XFB_HEIGHT / 2) {
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
