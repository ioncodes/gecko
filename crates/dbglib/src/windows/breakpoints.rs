use egui::{Context, Grid, ScrollArea};
use egui_phosphor::regular as icons;

use crate::Debugger;

pub fn show_breakpoints(ctx: &Context, open: &mut bool, debugger: &mut Debugger, addr_input: &mut String) {
    egui::Window::new("Breakpoints").open(open).show(ctx, |ui| {
        ui.horizontal(|ui| {
            egui::TextEdit::singleline(addr_input)
                .hint_text("address")
                .desired_width(100.0)
                .font(egui::TextStyle::Monospace)
                .show(ui);

            if ui.button(format!("{} Add", icons::PLUS)).clicked() {
                let s = addr_input.trim().trim_start_matches("0x");
                if let Ok(addr) = u32::from_str_radix(s, 16) {
                    debugger.add_breakpoint(addr);
                    addr_input.clear();
                }
            }
        });

        ui.separator();

        if debugger.breakpoints().is_empty() {
            ui.label("No breakpoints.");
            return;
        }

        let mut toggle_index: Option<usize> = None;
        let mut remove_index: Option<usize> = None;

        ScrollArea::vertical().show(ui, |ui| {
            Grid::new("breakpoint_list")
                .num_columns(3)
                .striped(true)
                .show(ui, |ui| {
                    for (i, bp) in debugger.breakpoints().iter().enumerate() {
                        let mut enabled = bp.enabled;
                        if ui.checkbox(&mut enabled, "").changed() {
                            toggle_index = Some(i);
                        }

                        ui.monospace(format!("{:#010X}", bp.addr));

                        if ui.button(icons::TRASH).clicked() {
                            remove_index = Some(i);
                        }

                        ui.end_row();
                    }
                });
        });

        if let Some(i) = toggle_index {
            debugger.toggle_breakpoint(i);
        }
        if let Some(i) = remove_index {
            debugger.remove_breakpoint(i);
        }
    });
}
