use egui::{Color32, Context, Grid, RichText, ScrollArea};
use gecko::flipper::dsp::Dsp;

use super::token_color;

/// Return the IMEM byte slice starting at the given DSP word address,
/// resolving IRAM (0x0000-0x0FFF) vs IROM (0x8000-0x8FFF).
fn imem_slice(dsp: &Dsp, word_addr: u16) -> Option<&[u8]> {
    match word_addr {
        0x0000..0x1000 => {
            let off = (word_addr as usize) * 2;
            dsp.iram.get(off..)
        }
        0x8000..0x9000 => {
            let off = (word_addr as usize - 0x8000) * 2;
            dsp.irom.get(off..)
        }
        _ => None,
    }
}

pub fn show_dsp(ctx: &Context, open: &mut bool, dsp: &Dsp) {
    egui::Window::new("DSP").open(open).show(ctx, |ui| {
        Grid::new("dsp_special_regs")
            .num_columns(4)
            .striped(true)
            .show(ui, |ui| {
                ui.label("PC");
                ui.monospace(format!("{:#06X}", dsp.registers.pc));
                ui.label("Halt");
                ui.label(if dsp.csr.halt() { "yes" } else { "no" });
                ui.end_row();

                ui.label("Reset");
                ui.label(if dsp.csr.reset() { "yes" } else { "no" });
                ui.label("CSR");
                ui.monospace(format!("{:#06X}", dsp.csr.raw()));
                ui.end_row();

                let mbox_to_dsp = ((dsp.mailbox_to_dsp_hi.raw() as u32) << 16) | dsp.mailbox_to_dsp_lo.raw() as u32;
                let mbox_to_cpu = ((dsp.mailbox_to_cpu_hi.raw() as u32) << 16) | dsp.mailbox_to_cpu_lo.raw() as u32;

                ui.label("CPU->DSP");
                ui.monospace(format!("{:#010X}", mbox_to_dsp));
                ui.label("DSP->CPU");
                ui.monospace(format!("{:#010X}", mbox_to_cpu));
                ui.end_row();

                #[cfg(not(target_arch = "wasm32"))]
                if ui.button("Dump DSP").clicked() {
                    let mut dump = Vec::new();
                    dump.extend_from_slice(&dsp.iram[..]);
                    dump.extend_from_slice(&dsp.dram[..]);
                    std::fs::write("dsp_dump.bin", dump).expect("Failed to write DSP dump");
                }
                ui.end_row();
            });

        ui.separator();

        ScrollArea::vertical()
            .id_salt("dsp_disasm_scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                Grid::new("dsp_disasm_grid")
                    .num_columns(4)
                    .min_col_width(0.0)
                    .striped(true)
                    .show(ui, |ui| {
                        let mut addr = dsp.registers.pc;
                        for _ in 0..20 {
                            let mem = imem_slice(dsp, addr);
                            let Some(mem) = mem else { break };

                            let (text, words) = match disasm::dsp::GcDspInstruction::decode(mem) {
                                Some((insn, byte_len)) => (insn.to_string(), (byte_len / 2) as u16),
                                None => {
                                    let raw = u16::from_be_bytes([mem[0], mem[1]]);
                                    (format!(".word {:#06X}", raw), 1)
                                }
                            };

                            let is_pc = addr == dsp.registers.pc;

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
                                let w_off = (i as usize) * 2;
                                if w_off + 1 < mem.len() {
                                    let w = u16::from_be_bytes([mem[w_off], mem[w_off + 1]]);
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

                            addr += words;
                        }
                    });
            });
    });
}
