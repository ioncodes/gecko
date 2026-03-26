use egui::{Context, Grid, ScrollArea};
use gecko::mmio::Mmio;

pub fn show_mmio(ctx: &Context, open: &mut bool, base: &mut u32, addr_input: &mut String, mmio: &Mmio) {
    egui::Window::new("Memory").open(open).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Address:");
            let resp = ui.add(
                egui::TextEdit::singleline(addr_input)
                    .desired_width(80.0)
                    .font(egui::TextStyle::Monospace),
            );
            let go = ui.button("Go").clicked();
            if go || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                let s = addr_input.trim().trim_start_matches("0x").trim_start_matches("0X");
                if let Ok(addr) = u32::from_str_radix(s, 16) {
                    *base = addr & !0xF;
                }
            }
            if ui.button(egui_phosphor::regular::CARET_LEFT).clicked() {
                *base = base.saturating_sub(256);
                *addr_input = format!("{:08X}", base);
            }
            if ui.button(egui_phosphor::regular::CARET_RIGHT).clicked() {
                *base = base.saturating_add(256);
                *addr_input = format!("{:08X}", base);
            }
        });

        ui.separator();

        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            Grid::new("mem_grid").num_columns(3).striped(true).show(ui, |ui| {
                for row in 0u32..16 {
                    let addr = base.wrapping_add(row * 16);

                    ui.monospace(format!("{:08X}", addr));

                    let mut hex = String::with_capacity(16 * 3 + 1);
                    let mut ascii = String::with_capacity(16);
                    for col in 0u32..16 {
                        if col == 8 {
                            hex.push(' ');
                        }
                        let byte = mmio.virt_read_u8(addr.wrapping_add(col));
                        hex.push_str(&format!("{:02X}", byte));
                        if col < 15 {
                            hex.push(' ');
                        }
                        ascii.push(if byte.is_ascii_graphic() { byte as char } else { '.' });
                    }
                    ui.monospace(&hex);
                    ui.monospace(&ascii);
                    ui.end_row();
                }
            });
        });
    });
}
