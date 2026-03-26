use egui::{Context, Grid, RichText, ScrollArea};
use gecko::flipper::gx::GraphicsProcessor;
use gecko::flipper::gx::draw::TextureDescriptor;
use gecko::mmio::Mmio;

fn texture_preview(ui: &mut egui::Ui, tex: &TextureDescriptor, ram: &[u8]) {
    let rgba = backend_wgpu::texture::decode_to_rgba(ram, tex);
    let size = [tex.width as usize, tex.height as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
    let handle = ui.ctx().load_texture(
        format!("tex_preview_{:08x}", tex.ram_addr),
        image,
        egui::TextureOptions::NEAREST,
    );
    let scale = 256.0 / tex.width.max(tex.height) as f32;
    let display = egui::vec2(tex.width as f32 * scale, tex.height as f32 * scale);
    ui.image(egui::load::SizedTexture::new(handle.id(), display));
    ui.separator();
    ui.label(format!("RAM: 0x{:08X}", tex.ram_addr));
    ui.label(format!("Wrap S: {:?}  T: {:?}", tex.wrap_s, tex.wrap_t));
    ui.label(format!("Mag: {:?}  Min: {:?}", tex.mag_filter, tex.min_filter));
}

pub fn show_gx(ctx: &Context, open: &mut bool, gx: &GraphicsProcessor, mmio: &Mmio) {
    egui::Window::new("GX").open(open).show(ctx, |ui| {
        let dc = &gx.draw_commands;

        ScrollArea::vertical().show(ui, |ui| {
            // Summary
            ui.horizontal(|ui| {
                ui.strong(format!("{} draw calls", dc.commands.len()));
            });
            ui.separator();

            // Transform
            ui.collapsing("Projection", |ui| {
                Grid::new("proj").num_columns(4).show(ui, |ui| {
                    for row in 0..4 {
                        for col in 0..4 {
                            ui.monospace(format!("{:+9.4}", dc.projection.0[col][row]));
                        }
                        ui.end_row();
                    }
                });
            });

            // Draw Calls (each with per-draw state)
            ui.collapsing(format!("Draw Calls ({})", dc.commands.len()), |ui| {
                ScrollArea::vertical()
                    .id_salt("draw_calls")
                    .max_height(500.0)
                    .show(ui, |ui| {
                        for (i, call) in dc.commands.iter().enumerate() {
                            let has_tex = call.textures[0].is_some();
                            let heading = RichText::new(format!(
                                "[{i:>3}]  {:?}  x  {} verts  tev={}  {}",
                                call.primitive,
                                call.vertices.len(),
                                call.num_tev_stages,
                                if has_tex { "tex" } else { "no-tex" },
                            ))
                            .monospace();

                            ui.collapsing(heading, |ui| {
                                // Texture
                                if let Some(tex) = &call.textures[0] {
                                    ui.collapsing("Texture", |ui| {
                                        ui.monospace(format!(
                                            "{:?} {}x{} @ 0x{:08X}",
                                            tex.format, tex.width, tex.height, tex.ram_addr
                                        ))
                                        .on_hover_ui(|ui| texture_preview(ui, tex, &mmio.ram));
                                    });
                                }

                                // TEV
                                ui.collapsing(format!("TEV ({} stages)", call.num_tev_stages), |ui| {
                                    for stage in 0..call.num_tev_stages as usize {
                                        let color = call.tev_color_env[stage];
                                        let alpha = call.tev_alpha_env[stage];

                                        ui.collapsing(format!("Stage {stage}"), |ui| {
                                            Grid::new(format!("tev_{i}_{stage}")).num_columns(2).striped(true).show(
                                                ui,
                                                |ui| {
                                                    ui.label("Color A");
                                                    ui.monospace(format!("{}", color.a()));
                                                    ui.end_row();
                                                    ui.label("Color B");
                                                    ui.monospace(format!("{}", color.b()));
                                                    ui.end_row();
                                                    ui.label("Color C");
                                                    ui.monospace(format!("{}", color.c()));
                                                    ui.end_row();
                                                    ui.label("Color D");
                                                    ui.monospace(format!("{}", color.d()));
                                                    ui.end_row();
                                                    ui.label("Color Dest");
                                                    ui.monospace(format!("{}", color.dest()));
                                                    ui.end_row();

                                                    ui.label("Alpha A");
                                                    ui.monospace(format!("{}", alpha.a()));
                                                    ui.end_row();
                                                    ui.label("Alpha B");
                                                    ui.monospace(format!("{}", alpha.b()));
                                                    ui.end_row();
                                                    ui.label("Alpha C");
                                                    ui.monospace(format!("{}", alpha.c()));
                                                    ui.end_row();
                                                    ui.label("Alpha D");
                                                    ui.monospace(format!("{}", alpha.d()));
                                                    ui.end_row();
                                                    ui.label("Alpha Dest");
                                                    ui.monospace(format!("{}", alpha.dest()));
                                                    ui.end_row();
                                                },
                                            );
                                        });
                                    }
                                });

                                // Blend / Depth
                                ui.collapsing("Output (PE)", |ui| {
                                    Grid::new(format!("pe_{i}"))
                                        .num_columns(2)
                                        .striped(true)
                                        .show(ui, |ui| {
                                            let bm = call.bp_blend_mode;
                                            ui.label("Blend");
                                            if bm.blend_enable() {
                                                ui.monospace(format!(
                                                    "{:?} -> {:?}{}",
                                                    bm.src_factor(),
                                                    bm.dst_factor(),
                                                    if bm.subtract() { " (sub)" } else { "" },
                                                ));
                                            } else {
                                                ui.label("disabled");
                                            }
                                            ui.end_row();

                                            let zm = call.bp_zmode;
                                            ui.label("Depth");
                                            if zm.enable() {
                                                ui.monospace(format!("{:?} write={}", zm.func(), zm.update_enable()));
                                            } else {
                                                ui.label("disabled");
                                            }
                                            ui.end_row();
                                        });
                                });

                                // Modelview
                                ui.collapsing("Modelview", |ui| {
                                    Grid::new(format!("mv_{i}")).num_columns(4).show(ui, |ui| {
                                        for row in 0..4 {
                                            for col in 0..4 {
                                                ui.monospace(format!("{:+8.3}", call.modelview.0[col][row]));
                                            }
                                            ui.end_row();
                                        }
                                    });
                                });

                                // Vertices
                                let preview = call.vertices.len().min(4);
                                for (vi, v) in call.vertices.iter().take(preview).enumerate() {
                                    ui.monospace(format!(
                                        "v{vi}  pos ({:+.3}, {:+.3}, {:+.3})",
                                        v.position[0], v.position[1], v.position[2],
                                    ));
                                    ui.monospace(format!(
                                        "    col ({:.2}, {:.2}, {:.2}, {:.2})",
                                        v.color0[0], v.color0[1], v.color0[2], v.color0[3],
                                    ));
                                    if let Some(uv) = v.texcoords[0] {
                                        ui.monospace(format!("    uv  ({:+.4}, {:+.4})", uv[0], uv[1]));
                                    }
                                }
                                if call.vertices.len() > preview {
                                    ui.label(format!(
                                        "{} {} more vertices",
                                        egui_phosphor::regular::DOTS_THREE,
                                        call.vertices.len() - preview,
                                    ));
                                }
                            });
                        }
                    });
            });
        });
    });
}
