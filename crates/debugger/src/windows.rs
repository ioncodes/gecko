pub mod controls;
pub mod cpu;
pub mod dsp;
pub mod dvd;
pub mod exi;
pub mod gx;
pub mod irq;
pub mod mmio;

pub(crate) fn flag(ui: &mut egui::Ui, val: bool) {
    use egui::{Color32, RichText};
    let (icon, color) = if val {
        (egui_phosphor::regular::CHECK_CIRCLE, Color32::from_rgb(100, 220, 100))
    } else {
        (egui_phosphor::regular::CIRCLE, Color32::from_rgb(70, 70, 70))
    };
    ui.label(RichText::new(icon).color(color));
}
