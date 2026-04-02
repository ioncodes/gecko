pub mod callstack;
pub mod controls;
pub mod cpu;
pub mod dsp;
pub mod dvd;
pub mod exi;
pub mod gx;
pub mod irq;
pub mod lua;
pub mod mmio;

use disasm::tokenizer::AsmToken;
use egui::{Color32, RichText};

pub(crate) fn flag(ui: &mut egui::Ui, val: bool) {
    let (icon, color) = if val {
        (egui_phosphor::regular::CHECK_CIRCLE, Color32::from_rgb(100, 220, 100))
    } else {
        (egui_phosphor::regular::CIRCLE, Color32::from_rgb(70, 70, 70))
    };
    ui.label(RichText::new(icon).color(color));
}

pub(crate) fn token_color(token: &AsmToken<'_>) -> Option<Color32> {
    match token {
        AsmToken::Mnemonic(_) => Some(Color32::from_rgb(100, 180, 255)),
        AsmToken::Gpr(_) | AsmToken::Fpr(_) | AsmToken::CrField(_) | AsmToken::Spr(_) => {
            Some(Color32::from_rgb(255, 200, 100))
        }
        AsmToken::ImmSigned(_)
        | AsmToken::ImmUnsigned(_)
        | AsmToken::ImmHex(_)
        | AsmToken::Displacement(_)
        | AsmToken::AddrPrefix
        | AsmToken::ImmPrefix => Some(Color32::from_rgb(150, 220, 150)),
        AsmToken::BranchTarget(_) => Some(Color32::from_rgb(255, 150, 150)),
        AsmToken::Punct(_) | AsmToken::Text(_) => None,
    }
}
