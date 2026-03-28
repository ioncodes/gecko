use egui::{Align, Color32, Context, Grid, RichText, ScrollArea};
use gecko::flipper::dsp::Dsp;

pub fn show_dsp(ctx: &Context, open: &mut bool, dsp: &Dsp) {
    egui::Window::new("DSP")
        .open(open)
        .default_size(egui::vec2(500.0, 600.0))
        .show(ctx, |ui| {
            Grid::new("dsp_special_regs")
                .num_columns(4)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("PC");
                    ui.monospace(format!("{:#06X}", dsp.pc));
                    ui.label("Halt");
                    ui.label(if dsp.csr.halt() { "yes" } else { "no" });
                    ui.end_row();

                    ui.label("Reset");
                    ui.label(if dsp.csr.reset() { "yes" } else { "no" });
                    ui.label("CSR");
                    ui.monospace(format!("{:#06X}", dsp.csr.raw()));
                    ui.end_row();
                });

            ui.separator();

            ScrollArea::vertical().id_salt("dsp_disasm_scroll").show(ui, |ui| {
                Grid::new("dsp_disasm_grid")
                    .num_columns(4)
                    .min_col_width(0.0)
                    .striped(true)
                    .show(ui, |ui| {
                        let mut addr = dsp.pc;
                        for _ in 0..20 {
                            let off = addr as usize;
                            if off + 1 >= dsp.iram.len() {
                                break;
                            }

                            let (text, words) = match disasm::dsp::GcDspInstruction::decode(&dsp.iram[off..]) {
                                Some((insn, byte_len)) => (insn.to_string(), (byte_len / 2) as u16),
                                None => {
                                    let raw = u16::from_be_bytes([dsp.iram[off], dsp.iram[off + 1]]);
                                    (format!(".word {:#06X}", raw), 1)
                                }
                            };

                            let is_pc = addr == dsp.pc;

                            // PC indicator
                            if is_pc {
                                ui.label(
                                    RichText::new(egui_phosphor::regular::PLAY).color(Color32::from_rgb(120, 220, 120)),
                                );
                            } else {
                                ui.label("");
                            }

                            // Address
                            ui.monospace(format!("{:#06X}", addr));

                            // Raw bytes
                            let mut raw_str = String::new();
                            for i in 0..words {
                                let w_off = off + i as usize;
                                if w_off + 1 < dsp.iram.len() {
                                    let w = u16::from_be_bytes([dsp.iram[w_off], dsp.iram[w_off + 1]]);
                                    if !raw_str.is_empty() {
                                        raw_str.push(' ');
                                    }
                                    raw_str.push_str(&format!("{:04X}", w));
                                }
                            }
                            ui.label(
                                RichText::new(raw_str)
                                    .monospace()
                                    .color(Color32::from_rgb(100, 100, 100)),
                            );

                            // Disassembly
                            ui.monospace(&text);
                            ui.end_row();

                            if is_pc {
                                ui.scroll_to_cursor(Some(Align::Min));
                            }

                            addr += words;
                        }
                    });
            });
        });
}
