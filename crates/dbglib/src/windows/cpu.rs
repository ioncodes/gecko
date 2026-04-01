use egui::{Color32, Context, Grid, RichText, ScrollArea};
use gecko::cpu::Cpu;
use gecko::mmio::Mmio;
use image::symbols::SymbolTable;

use super::token_color;

pub fn show_cpu(ctx: &Context, open: &mut bool, cpu: &Cpu, mmio: &Mmio, symbols: Option<&SymbolTable>) {
    egui::Window::new("CPU").open(open).show(ctx, |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_min_width(350.0);
                ui.set_max_width(350.0);
                Grid::new("special_regs").num_columns(5).striped(true).show(ui, |ui| {
                    ui.label("PC");
                    ui.monospace(format!("{:#010X}", cpu.pc));
                    ui.allocate_space(egui::vec2(16.0, 0.0));
                    ui.label("CTR");
                    ui.monospace(format!("{:#010X}", cpu.spr.ctr));
                    ui.end_row();

                    ui.label("LR");
                    ui.monospace(format!("{:#010X}", cpu.spr.lr));
                    ui.allocate_space(egui::vec2(16.0, 0.0));
                    ui.label("MSR");
                    ui.monospace(format!("{:#010X}", cpu.msr.raw()));
                    ui.end_row();

                    ui.label("XER");
                    ui.monospace(format!("{:#010X}", cpu.spr.xer.raw()));
                    ui.allocate_space(egui::vec2(16.0, 0.0));
                    ui.label("FPSCR");
                    ui.monospace(format!("{:#010X}", cpu.fpscr));
                    ui.end_row();
                });

                ui.separator();

                ui.scope(|ui| {
                    Grid::new("cr_fields")
                        .num_columns(9)
                        .spacing([1.0, 1.0])
                        .striped(true)
                        .show(ui, |ui| {
                            ui.label("");
                            for i in 0..8u8 {
                                ui.label(format!("CR{i}"));
                            }
                            ui.end_row();

                            for (name, getter) in [
                                ("LT", (|i: u8, c: &Cpu| c.cr.get_field(i).lt()) as fn(u8, &Cpu) -> bool),
                                ("GT", (|i: u8, c: &Cpu| c.cr.get_field(i).gt()) as fn(u8, &Cpu) -> bool),
                                ("EQ", (|i: u8, c: &Cpu| c.cr.get_field(i).eq()) as fn(u8, &Cpu) -> bool),
                                ("SO", (|i: u8, c: &Cpu| c.cr.get_field(i).so()) as fn(u8, &Cpu) -> bool),
                            ] {
                                ui.label(name);
                                for i in 0..8u8 {
                                    let mut val = getter(i, cpu);
                                    ui.add_enabled(false, egui::Checkbox::without_text(&mut val));
                                }
                                ui.end_row();
                            }
                        });
                });

                ui.separator();

                let pc = cpu.pc;
                let start = pc.saturating_sub(16 * 4);
                let end = pc.saturating_add(16 * 4);

                ScrollArea::vertical()
                    .id_salt("disasm_scroll")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                    .show(ui, |ui| {
                        Grid::new("disasm_grid")
                            .num_columns(4)
                            .min_col_width(0.0)
                            .striped(true)
                            .show(ui, |ui| {
                                let mut addr = start;
                                while addr <= end {
                                    // Show function label if this address is a function entry
                                    if let Some(sym) = symbols.and_then(|s| s.lookup_exact(addr)) {
                                        if sym.kind == image::symbols::SymbolKind::Func {
                                            ui.label("");
                                            ui.label("");
                                            ui.label("");
                                            ui.label(
                                                RichText::new(format!("{}:", sym.name))
                                                    .monospace()
                                                    .color(Color32::from_rgb(220, 180, 80)),
                                            );
                                            ui.end_row();
                                        }
                                    }

                                    let raw = mmio.virt_read_u32(addr);
                                    let text = disasm::gekko::GekkoInstruction::decode(&raw.to_be_bytes())
                                        .map(|(insn, _)| insn.to_string())
                                        .unwrap_or_else(|| format!(".word {:#010X}", raw));

                                    let is_pc = addr == pc;

                                    if is_pc {
                                        ui.label(
                                            RichText::new(egui_phosphor::regular::PLAY)
                                                .color(Color32::from_rgb(120, 220, 120)),
                                        );
                                    } else {
                                        ui.label("");
                                    }

                                    ui.monospace(format!("{:#010X}", addr));

                                    ui.label(
                                        RichText::new(format!("{:08X}", raw))
                                            .monospace()
                                            .color(Color32::from_rgb(100, 100, 100)),
                                    );

                                    let tokens = disasm::tokenizer::tokenize(&text);
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 0.0;
                                        for token in &tokens {
                                            let mut rt = RichText::new(token.to_string()).monospace();
                                            if let Some(color) = token_color(token) {
                                                rt = rt.color(color);
                                            }
                                            ui.label(rt);
                                        }
                                    });

                                    ui.end_row();

                                    addr = addr.wrapping_add(4);
                                }
                            });
                    });
            });

            ui.separator();

            ui.vertical(|ui| {
                ui.label("GPRs");
                ScrollArea::vertical()
                    .id_salt("gprs_scroll")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                    .show(ui, |ui| {
                        Grid::new("gprs").num_columns(2).striped(true).show(ui, |ui| {
                            for (i, &val) in cpu.gprs.iter().enumerate() {
                                ui.label(format!("r{i:<2}"));
                                ui.monospace(format!("{:#010X}", val));
                                ui.end_row();
                            }
                        });
                    });
            });

            ui.separator();

            ui.vertical(|ui| {
                ui.label("FPRs");
                ScrollArea::vertical()
                    .id_salt("fprs_scroll")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                    .show(ui, |ui| {
                        Grid::new("fprs").num_columns(2).striped(true).show(ui, |ui| {
                            for (i, &val) in cpu.fprs.iter().enumerate() {
                                ui.label(format!("f{i:<2}"));
                                ui.monospace(format!("{:+.6e}", val));
                                ui.end_row();
                            }
                        });
                    });
            });
        });
    });
}
