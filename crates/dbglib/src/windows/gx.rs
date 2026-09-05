use egui::{Context, Grid, ScrollArea};
use gecko::SystemId;
use gecko::flipper::gx::GraphicsProcessor;
use gecko::flipper::gx::draw::{TextureDescriptor, TlutRef};
use gecko::mmio::Mmio;
use glam::{Mat3, Mat4, Vec3};

pub struct Freecam {
    pub enabled: bool,
    pub speed: f32,
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    sent_disable: bool,
}

impl Default for Freecam {
    fn default() -> Self {
        Freecam {
            enabled: false,
            speed: 50.0,
            pos: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            sent_disable: true,
        }
    }
}

impl Freecam {
    pub fn reset(&mut self) {
        self.pos = Vec3::ZERO;
        self.yaw = 0.0;
        self.pitch = 0.0;
    }

    pub fn update_input(&mut self, ctx: &Context) {
        if !self.enabled {
            return;
        }

        let pointer_free = !ctx.egui_wants_pointer_input();
        let keys_free = !ctx.egui_wants_keyboard_input();

        ctx.input(|i| {
            if pointer_free && i.pointer.secondary_down() {
                let d = i.pointer.delta();
                self.yaw -= d.x * 0.005;
                self.pitch = (self.pitch - d.y * 0.005).clamp(-1.55, 1.55);
            }

            if pointer_free && i.smooth_scroll_delta.y != 0.0 {
                self.speed = (self.speed * 1.2f32.powf(i.smooth_scroll_delta.y / 40.0)).clamp(0.01, 1e6);
            }

            if !keys_free {
                return;
            }

            let mut dir = Vec3::ZERO;
            if i.key_down(egui::Key::W) {
                dir.z -= 1.0;
            }
            if i.key_down(egui::Key::S) {
                dir.z += 1.0;
            }
            if i.key_down(egui::Key::A) {
                dir.x -= 1.0;
            }
            if i.key_down(egui::Key::D) {
                dir.x += 1.0;
            }
            if i.key_down(egui::Key::Q) {
                dir.y -= 1.0;
            }
            if i.key_down(egui::Key::E) {
                dir.y += 1.0;
            }
            if dir == Vec3::ZERO {
                return;
            }

            let boost = if i.modifiers.shift { 5.0 } else { 1.0 };
            let dt = i.stable_dt.min(0.1);
            self.pos += self.rotation() * dir.normalize() * self.speed * boost * dt;
        });
    }

    pub fn take_action(&mut self) -> Option<([[f32; 4]; 4], bool)> {
        if self.enabled {
            self.sent_disable = false;

            let view = Mat4::from_mat3(self.rotation().transpose()) * Mat4::from_translation(-self.pos);
            Some((view.to_cols_array_2d(), true))
        } else if !self.sent_disable {
            self.sent_disable = true;
            Some((Mat4::IDENTITY.to_cols_array_2d(), false))
        } else {
            None
        }
    }

    fn rotation(&self) -> Mat3 {
        Mat3::from_rotation_y(self.yaw) * Mat3::from_rotation_x(self.pitch)
    }
}

fn texture_preview(ui: &mut egui::Ui, tex: &TextureDescriptor, ram: &[u8], palette: &[u16], tlut: TlutRef) {
    let rgba = gecko::flipper::gx::texture::decode_to_rgba(ram, tex, palette, tlut.format);
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

pub fn show_gx<const SYSTEM: SystemId>(
    ctx: &Context,
    open: &mut bool,
    gx: &GraphicsProcessor,
    mmio: &Mmio<SYSTEM>,
    invalidate_caches: &mut bool,
    dump_textures: &mut bool,
    freecam: &mut Freecam,
) {
    egui::Window::new("GX").open(open).show(ctx, |ui| {
        ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Invalidate Caches").clicked() {
                    *invalidate_caches = true;
                }
                #[cfg(not(target_arch = "wasm32"))]
                if ui.button("Dump Textures").clicked() {
                    *dump_textures = true;
                }
                #[cfg(target_arch = "wasm32")]
                let _ = dump_textures;
                ui.label(format!("{} cached", gx.texture_hashes.len()));
            });
            ui.separator();

            // Freecam
            ui.collapsing("Freecam", |ui| {
                ui.checkbox(&mut freecam.enabled, "Unlock camera");

                ui.horizontal(|ui| {
                    ui.label("Speed");
                    ui.add(egui::Slider::new(&mut freecam.speed, 0.01..=1_000_000.0).logarithmic(true));
                });

                if ui.button("Reset view").clicked() {
                    freecam.reset();
                }

                ui.label("Hold RMB to look, WASD + QE to fly, Shift boosts, scroll tunes speed");
            });

            // Projection matrix
            ui.collapsing("Projection", |ui| {
                Grid::new("proj").num_columns(4).show(ui, |ui| {
                    for row in 0..4 {
                        for col in 0..4 {
                            ui.monospace(format!("{:+9.4}", gx.projection.0[col][row]));
                        }
                        ui.end_row();
                    }
                });
            });

            // Viewport / Scissor
            ui.collapsing("Viewport / Scissor", |ui| {
                Grid::new("vp_sc").num_columns(2).striped(true).show(ui, |ui| {
                    let vp = &gx.cur_viewport;
                    ui.label("Viewport");
                    ui.monospace(format!(
                        "({:.0}, {:.0}) {}x{} depth [{:.2}, {:.2}]",
                        vp.x, vp.y, vp.w, vp.h, vp.min_depth, vp.max_depth
                    ));
                    ui.end_row();

                    let sc = &gx.cur_scissor;
                    ui.label("Scissor");
                    ui.monospace(format!("({}, {}) {}x{}", sc.x, sc.y, sc.w, sc.h));
                    ui.end_row();

                    ui.label("Scissor Offset");
                    ui.monospace(format!("({}, {})", gx.cur_scissor_offset_x, gx.cur_scissor_offset_y));
                    ui.end_row();
                });
            });

            // Blend / Depth
            ui.collapsing("Output (PE)", |ui| {
                Grid::new("pe_state").num_columns(2).striped(true).show(ui, |ui| {
                    let bm = gx.cur_blend_mode;
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

                    let zm = gx.cur_zmode;
                    ui.label("Depth");
                    if zm.enable() {
                        ui.monospace(format!("{:?} write={}", zm.func(), zm.update_enable()));
                    } else {
                        ui.label("disabled");
                    }
                    ui.end_row();

                    let ac = gx.cur_alpha_compare;
                    ui.label("Alpha Compare");
                    ui.monospace(format!("{:?} / {:?}", ac.comp0(), ac.comp1()));
                    ui.end_row();
                });
            });

            // TEV configuration
            ui.collapsing(format!("TEV ({} stages)", gx.cur_num_tev_stages), |ui| {
                for stage in 0..gx.cur_num_tev_stages as usize {
                    let color = gx.cur_tev_color_env[stage];
                    let alpha = gx.cur_tev_alpha_env[stage];

                    ui.collapsing(format!("Stage {stage}"), |ui| {
                        Grid::new(format!("tev_{stage}"))
                            .num_columns(2)
                            .striped(true)
                            .show(ui, |ui| {
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
                            });
                    });
                }
            });

            // Bound textures
            ui.collapsing("Textures", |ui| {
                for (slot, tex) in gx.cur_textures.iter().enumerate() {
                    if let Some(tex) = tex {
                        let heading = format!(
                            "Slot {slot}: {:?} {}x{} @ 0x{:08X}",
                            tex.format, tex.width, tex.height, tex.ram_addr
                        );
                        let tlut = gx.cur_tluts[slot];
                        let base = (tlut.tmem_offset as usize) * 256;
                        let palette = gx.palette_mem.get(base..).unwrap_or(&[]);
                        ui.collapsing(heading, |ui| {
                            texture_preview(ui, tex, &mmio.ram, palette, tlut);
                        });
                    }
                }
            });
        });
    });
}
