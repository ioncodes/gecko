pub mod regs;

use crate::flipper::pi::InterruptFlag;
use crate::system::{System, SystemId};

pub struct PixelEngine {
    pub zconf: regs::ZConfig,
    pub alphaconf: regs::AlphaConfig,
    pub dst_alphaconf: regs::DstAlphaConfig,
    pub alphamode: regs::AlphaMode,
    pub alpharead: regs::AlphaRead,
    pub sr: regs::InterruptStatus,
    pub token: regs::Token,
}

impl PixelEngine {
    pub fn new() -> Self {
        Self {
            zconf: regs::ZConfig::from_raw(0),
            alphaconf: regs::AlphaConfig::from_raw(0),
            dst_alphaconf: regs::DstAlphaConfig::from_raw(0),
            alphamode: regs::AlphaMode::from_raw(0),
            alpharead: regs::AlphaRead::from_raw(0),
            sr: regs::InterruptStatus::from_raw(0),
            token: regs::Token::from_raw(0),
        }
    }

    #[inline(always)]
    pub fn finish_interrupt_active(&self) -> bool {
        self.sr.pe_finish_enable() && self.sr.pe_finish()
    }

    #[inline(always)]
    pub fn token_interrupt_active(&self) -> bool {
        self.sr.pe_token_enable() && self.sr.pe_token()
    }

    pub fn signal_finish(&mut self) {
        self.sr = self.sr.with_pe_finish(true);
    }

    pub fn set_token(&mut self, token: u16) {
        self.token = regs::Token::from_raw(token.into());
    }

    pub fn signal_token(&mut self, token: u16) {
        self.token = regs::Token::from_raw(token.into());
        self.sr = self.sr.with_pe_token(true);
    }
}

crate::mmio_device_dispatch! {
    read = pe_read,
    write = pe_write,
    registers = [
        regs::ZConfig,
        regs::AlphaConfig,
        regs::DstAlphaConfig,
        regs::AlphaMode,
        regs::AlphaRead,
        regs::InterruptStatus,
        regs::Token,
    ],
}

#[inline(always)]
pub fn refresh_interrupts<const SYSTEM: SystemId>(sys: &mut System<SYSTEM>) {
    if sys.pe.token_interrupt_active() {
        sys.pi.assert_interrupt(InterruptFlag::PeToken);
    } else {
        sys.pi.clear_interrupt(InterruptFlag::PeToken);
    }

    if sys.pe.finish_interrupt_active() {
        sys.pi.assert_interrupt(InterruptFlag::PeFinish);
    } else {
        sys.pi.clear_interrupt(InterruptFlag::PeFinish);
    }
}
