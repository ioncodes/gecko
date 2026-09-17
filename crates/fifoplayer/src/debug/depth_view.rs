use super::geometry::{Geometry, GeometryCapture, ProjectedVertex, clip_polygon};
use egui::{Color32, Pos2, Sense, Stroke, Vec2, vec2};
use gecko::flipper::gx::constants::{DEPTH_24_BIT_MAX, EFB_HEIGHT, EFB_WIDTH};
use gecko::flipper::gx::depth::DepthPlane;
use gecko::flipper::gx::regs::CompareFunc;
use glam::{Mat3, Vec3};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const OWN: Color32 = Color32::from_rgb(70, 220, 255);
const FROZEN: Color32 = Color32::from_rgb(120, 240, 160);
const REFERENCE: Color32 = Color32::GOLD;
const SCENE: Color32 = Color32::from_rgb(135, 155, 185);

pub struct DepthView {
    yaw: f32,
    pitch: f32,
    zoom: f32,
    pan: Vec2,
    expand_far: bool,
    stretch: f32,
    show_own: bool,
    show_frozen: bool,
    show_plane: bool,
    show_scene: bool,
    writers_only: bool,
    show_textures: bool,
    pub show_test: bool,
    textures: HashMap<(usize, egui::TextureWrapMode), egui::TextureHandle>,
    cached_draws: Vec<Arc<Geometry>>,
    polygons: Vec<Polygon>,
}

struct Polygon {
    draw: usize,
    points: Vec<ProjectedVertex>,
}

impl Default for DepthView {
    fn default() -> Self {
        Self {
            yaw: -0.85,
            pitch: 0.25,
            zoom: 1.0,
            pan: Vec2::ZERO,
            expand_far: true,
            stretch: 1.0,
            show_own: true,
            show_frozen: true,
            show_plane: true,
            show_scene: true,
            writers_only: false,
            show_textures: true,
            show_test: true,
            textures: HashMap::new(),
            cached_draws: Vec::new(),
            polygons: Vec::new(),
        }
    }
}

fn depth_fraction(z: f32, expand_far: bool) -> f32 {
    let z = (z / DEPTH_24_BIT_MAX).clamp(0.0, 1.0);

    if expand_far {
        (65537.0 / (1.0 + (1.0 - z) * 65536.0)).ln() / 65537.0_f32.ln()
    } else {
        z
    }
}

fn on_plane(p: Vec3, plane: DepthPlane) -> Vec3 {
    Vec3::new(p.x, p.y, plane.eval(p.x, p.y))
}

fn depth_passes(func: CompareFunc, incoming: u32, stored: u32) -> bool {
    match func {
        CompareFunc::Never => false,
        CompareFunc::Less => incoming < stored,
        CompareFunc::Equal => incoming == stored,
        CompareFunc::LessEqual => incoming <= stored,
        CompareFunc::Greater => incoming > stored,
        CompareFunc::NotEqual => incoming != stored,
        CompareFunc::GreaterEqual => incoming >= stored,
        CompareFunc::Always => true,
    }
}

fn compare_symbol(func: CompareFunc) -> &'static str {
    match func {
        CompareFunc::Never => "never",
        CompareFunc::Less => "<",
        CompareFunc::Equal => "=",
        CompareFunc::LessEqual => "≤",
        CompareFunc::Greater => ">",
        CompareFunc::NotEqual => "≠",
        CompareFunc::GreaterEqual => "≥",
        CompareFunc::Always => "always",
    }
}

fn probe_point(pointer: Pos2, xy: [Pos2; 3], vertices: [Vec3; 3]) -> Option<Vec3> {
    let matrix = Mat3::from_cols_array_2d(&xy.map(|p| [p.x, p.y, 1.0]));

    if matrix.determinant().abs() < 1e-6 {
        return None;
    }

    let weights = matrix.inverse() * Vec3::new(pointer.x, pointer.y, 1.0);

    if !weights.is_finite() || weights.min_element() < -1e-5 {
        return None;
    }

    let p = Mat3::from_cols(vertices[0], vertices[1], vertices[2]) * weights;
    let (x, y) = (p.x.floor(), p.y.floor());

    if !(0.0..EFB_WIDTH as f32).contains(&x) || !(0.0..EFB_HEIGHT as f32).contains(&y) {
        return None;
    }

    let plane = DepthPlane::from_points(vertices)?;

    Some(self::on_plane(Vec3::new(x + 0.5, y + 0.5, 0.0), plane))
}

fn draw_probe(painter: &egui::Painter, camera: &Camera, draw: &Geometry, original: Vec3) -> String {
    let incoming = if draw.frozen {
        let Some(reference) = &draw.reference else {
            return "Reference plane unavailable".into();
        };

        let incoming = self::on_plane(original, reference.plane);

        let original_xy = camera.project(original).0;
        let incoming_xy = camera.project(incoming).0;

        painter.extend(egui::Shape::dashed_line(
            &[original_xy, incoming_xy],
            Stroke::new(1.5, OWN),
            5.0,
            4.0,
        ));
        painter.circle_stroke(original_xy, 4.0, Stroke::new(1.5, OWN));

        incoming
    } else {
        original
    };

    let incoming_z = incoming.z.clamp(0.0, DEPTH_24_BIT_MAX) as u32;
    let incoming_xy = camera.project(incoming).0;

    let Some(depth) = draw.depth_before.get().and_then(Option::as_ref) else {
        painter.circle_stroke(incoming_xy, 4.0, Stroke::new(1.5, Color32::GRAY));
        return "Depth snapshot unavailable".into();
    };

    let (x, y) = (original.x as usize, original.y as usize);

    let Some(&stored_z) = depth.get(y * EFB_WIDTH as usize + x) else {
        return "Outside depth snapshot".into();
    };

    let func = draw.zmode.func();

    let (result, color, status) = if !draw.zmode.enable() {
        ("TEST OFF", Color32::GRAY, "Depth test off".to_string())
    } else if draw.z_texture {
        (
            "Z-TEXTURE",
            Color32::GRAY,
            "Z-texture: comparison unavailable".to_string(),
        )
    } else {
        let (result, color) = if self::depth_passes(func, incoming_z, stored_z) {
            ("PASS", Color32::LIGHT_GREEN)
        } else {
            ("FAIL", Color32::LIGHT_RED)
        };

        let status = match func {
            CompareFunc::Never | CompareFunc::Always => format!("{result} ({})", self::compare_symbol(func)),
            _ => format!(
                "{result} (incoming {incoming_z} {} stored {stored_z})",
                self::compare_symbol(func)
            ),
        };

        (result, color, status)
    };

    let stored_xy = camera.project(Vec3::new(original.x, original.y, stored_z as f32)).0;

    painter.line_segment([incoming_xy, stored_xy], Stroke::new(2.5, Color32::BLACK));
    painter.line_segment([incoming_xy, stored_xy], Stroke::new(1.5, color));
    painter.circle_filled(incoming_xy, 4.0, color);
    painter.circle_stroke(stored_xy, 5.0, Stroke::new(2.0, color));

    painter.text(
        incoming_xy + vec2(10.0, -10.0),
        egui::Align2::LEFT_BOTTOM,
        result,
        egui::FontId::monospace(12.0),
        color,
    );
    painter.text(
        stored_xy + vec2(10.0, 10.0),
        egui::Align2::LEFT_TOP,
        "Stored",
        egui::FontId::monospace(12.0),
        color,
    );

    status
}

fn surface(
    points: &[Pos2],
    fill: Color32,
    stroke: Stroke,
    texture: Option<(egui::TextureId, &[ProjectedVertex])>,
) -> Vec<egui::Shape> {
    let mut shapes = Vec::with_capacity(points.len() + 1);

    if fill != Color32::TRANSPARENT {
        let mut mesh = egui::Mesh::default();

        for &point in points {
            mesh.colored_vertex(point, fill);
        }

        for i in 1..points.len().saturating_sub(1) {
            mesh.add_triangle(0, i as u32, (i + 1) as u32);
        }

        if let Some((id, vertices)) = texture {
            mesh.texture_id = id;

            for (vertex, source) in mesh.vertices.iter_mut().zip(vertices) {
                vertex.uv = source.uv();
            }
        }

        shapes.push(egui::Shape::mesh(mesh));
    }

    for i in 0..points.len() {
        let edge = [points[i], points[(i + 1) % points.len()]];
        shapes.push(egui::Shape::line_segment(edge, stroke));
    }

    shapes
}

fn clip_efb(polygon: Vec<ProjectedVertex>) -> Vec<ProjectedVertex> {
    let bounds = [
        (0, 0.0, 1.0),
        (0, EFB_WIDTH as f32, -1.0),
        (1, 0.0, 1.0),
        (1, EFB_HEIGHT as f32, -1.0),
    ];

    clip_polygon(
        polygon,
        bounds,
        |p, (axis, boundary, sign)| (p.screen[axis] - boundary) * sign,
        ProjectedVertex::lerp,
    )
}

fn status_label(painter: &egui::Painter, position: Pos2, galley: Arc<egui::Galley>, color: Color32) {
    let backdrop = egui::Rect::from_min_size(position, galley.size()).expand(3.0);

    painter.rect_filled(backdrop, 2.0, Color32::from_black_alpha(200));
    painter.galley(position, galley, color);
}

struct Camera {
    center: Pos2,
    scale: f32,
    rotation: Mat3,
    stretch: f32,
    expand_far: bool,
}

impl Camera {
    fn new(view: &DepthView, rect: egui::Rect) -> Self {
        Self {
            center: rect.center() + view.pan,
            scale: rect.width().min(rect.height()) * 0.65 * view.zoom,
            rotation: Mat3::from_rotation_x(view.pitch) * Mat3::from_rotation_y(view.yaw),
            stretch: view.stretch,
            expand_far: view.expand_far,
        }
    }

    fn project(&self, p: Vec3) -> (Pos2, f32) {
        let x = p.x / EFB_WIDTH as f32 - 0.5;
        let y = (p.y - EFB_HEIGHT as f32 * 0.5) / EFB_WIDTH as f32;
        let z = (self::depth_fraction(p.z, self.expand_far) - 0.5) * self.stretch;

        let p = self.rotation * Vec3::new(x, y, z);

        (self.center + vec2(p.x, p.y) * self.scale, p.z)
    }
}

impl DepthView {
    fn scene_is_current(&self, capture: &GeometryCapture) -> bool {
        let total: usize = capture.draws.values().map(Vec::len).sum();

        if total != self.cached_draws.len() {
            return false;
        }

        self.cached_draws.iter().all(|draw| {
            capture
                .draws
                .get(&draw.location.offset)
                .and_then(|draws| draws.get(draw.subdraw))
                .is_some_and(|current| Arc::ptr_eq(current, draw))
        })
    }

    fn update_scene(&mut self, capture: &GeometryCapture, ctx: &egui::Context) {
        if self.scene_is_current(capture) {
            return;
        }

        let mut draws: Vec<_> = capture.draws.values().flatten().cloned().collect();
        draws.sort_by_key(|d| (d.location.row, d.subdraw));

        let mut used = HashSet::new();

        for draw in &draws {
            let Some(texture) = &draw.texture else {
                continue;
            };

            let key = (Arc::as_ptr(texture) as usize, draw.texture_wrap);
            used.insert(key);

            self.textures.entry(key).or_insert_with(|| {
                let options = egui::TextureOptions {
                    wrap_mode: draw.texture_wrap,
                    ..egui::TextureOptions::LINEAR
                };

                ctx.load_texture("FIFO draw texture", texture.clone(), options)
            });
        }

        self.textures.retain(|key, _| used.contains(key));

        self.polygons.clear();

        for (index, draw) in draws.iter().enumerate() {
            for &triangle in &draw.triangles {
                let points = self::clip_efb(draw.projected_vertices(triangle));

                if points.len() >= 3 && points.iter().all(|p| p.screen.is_finite()) {
                    self.polygons.push(Polygon { draw: index, points });
                }
            }
        }

        self.cached_draws = draws;
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, selected: Option<&Arc<Geometry>>) {
        ui.horizontal_wrapped(|ui| {
            for (label, yaw, pitch) in [
                ("Front", 0.0, 0.0),
                ("Side", -std::f32::consts::FRAC_PI_2, 0.0),
                ("Reset", -0.85, 0.25),
            ] {
                if ui.button(label).clicked() {
                    self.yaw = yaw;
                    self.pitch = pitch;
                    self.pan = Vec2::ZERO;
                    self.zoom = 1.0;
                }
            }

            ui.separator();

            ui.checkbox(&mut self.show_textures, "Textures");
            ui.checkbox(&mut self.show_own, egui::RichText::new("Original").color(OWN));

            ui.add_enabled_ui(selected.is_some_and(|d| d.frozen), |ui| {
                ui.checkbox(&mut self.show_frozen, egui::RichText::new("Frozen").color(FROZEN));
            });

            ui.checkbox(&mut self.show_plane, egui::RichText::new("Plane").color(REFERENCE));

            ui.checkbox(&mut self.show_test, "Depth test");

            ui.menu_button("Options", |ui| {
                ui.checkbox(&mut self.show_scene, "Scene geometry");
                ui.checkbox(&mut self.writers_only, "Depth writers only");

                ui.separator();

                ui.checkbox(&mut self.expand_far, "Log depth");
                ui.add(egui::Slider::new(&mut self.stretch, 0.2..=4.0).text("Z scale"));
            });
        });
    }

    fn handle_input(&mut self, ui: &egui::Ui, response: &egui::Response) {
        if response.dragged() {
            let delta = ui.input(|i| i.pointer.delta());

            if response.dragged_by(egui::PointerButton::Secondary) || ui.input(|i| i.modifiers.shift) {
                self.pan += delta;
            } else if response.dragged_by(egui::PointerButton::Primary) {
                self.yaw += delta.x * 0.008;
                self.pitch = (self.pitch - delta.y * 0.008).clamp(-1.5, 1.5);
            }
        }

        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            self.zoom = (self.zoom * (scroll * 0.002).exp()).clamp(0.1, 20.0);
        }
    }

    fn draw_bounds(painter: &egui::Painter, camera: &Camera) {
        let (w, h) = (EFB_WIDTH as f32, EFB_HEIGHT as f32);
        let corners = [
            Vec3::ZERO,
            Vec3::new(w, 0.0, 0.0),
            Vec3::new(w, h, 0.0),
            Vec3::new(0.0, h, 0.0),
        ];

        for z in [0.0, DEPTH_24_BIT_MAX] {
            let points = corners.map(|p| camera.project(p.with_z(z)).0);

            let stroke = Stroke::new(1.0, Color32::from_gray(65));
            painter.extend(self::surface(&points, Color32::TRANSPARENT, stroke, None));
        }

        for p in corners {
            let near = camera.project(p).0;
            let far = camera.project(p.with_z(DEPTH_24_BIT_MAX)).0;

            painter.line_segment([near, far], Stroke::new(1.0, Color32::from_gray(50)));
        }

        for (z, label) in [(0.0, "Z = 0 (near)"), (DEPTH_24_BIT_MAX, "Z = 1 (far)")] {
            let p = camera.project(Vec3::new(0.0, h, z)).0;

            painter.text(
                p + vec2(0.0, 10.0),
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::monospace(12.0),
                Color32::LIGHT_GRAY,
            );
        }
    }

    fn draw_reference(painter: &egui::Painter, camera: &Camera, draw: &Arc<Geometry>) {
        let reference = match (&draw.reference, draw.last_plane) {
            (Some(r), _) => (&*r.draw, r.triangle, r.plane),
            (None, Some((triangle, plane))) if !draw.frozen => (&**draw, triangle, plane),
            _ => return,
        };

        let (source, triangle, plane) = reference;
        let (w, h) = (EFB_WIDTH as f32, EFB_HEIGHT as f32);
        let grid_stroke = Stroke::new(0.8, REFERENCE.gamma_multiply(0.35));

        for i in 0..=8 {
            let x = w * i as f32 / 8.0;
            let y = h * i as f32 / 8.0;

            for segment in [
                [Vec3::new(x, 0.0, 0.0), Vec3::new(x, h, 0.0)],
                [Vec3::new(0.0, y, 0.0), Vec3::new(w, y, 0.0)],
            ] {
                let line = segment.map(|p| camera.project(self::on_plane(p, plane)).0);
                painter.line_segment(line, grid_stroke);
            }
        }

        let xy: Vec<_> = source
            .projected_vertices(triangle)
            .iter()
            .map(|p| camera.project(p.screen).0)
            .collect();

        if xy.is_empty() {
            return;
        }

        painter.extend(self::surface(
            &xy,
            Color32::TRANSPARENT,
            Stroke::new(2.0, REFERENCE),
            None,
        ));

        let center = (xy.iter().fold(Vec2::ZERO, |sum, p| sum + p.to_vec2()) / xy.len() as f32).to_pos2();
        let label = center + vec2(20.0, 26.0);

        painter.circle_stroke(center, 6.0, Stroke::new(1.5, REFERENCE));
        painter.line_segment([center, label], Stroke::new(1.0, REFERENCE));
        painter.text(
            label,
            egui::Align2::LEFT_TOP,
            format!("Reference {:06X}", source.location.offset),
            egui::FontId::monospace(12.0),
            REFERENCE,
        );
    }

    fn draw_bound_labels(painter: &egui::Painter, rect: egui::Rect, selected_bounds: [egui::Rect; 2]) {
        let area = rect.shrink(6.0);

        for (bounds, text, color) in [
            (selected_bounds[0], "Original depth", OWN),
            (selected_bounds[1], "Frozen depth", FROZEN),
        ] {
            if !bounds.is_finite() {
                continue;
            }

            let label = painter.layout_no_wrap(text.into(), egui::FontId::monospace(12.0), color);
            let size = label.size();

            let x = bounds.center().x - size.x * 0.5;
            let y = bounds.top() - size.y - 8.0;

            let position = egui::pos2(
                x.clamp(area.left(), (area.right() - size.x).max(area.left())),
                y.clamp(area.top(), (area.bottom() - size.y).max(area.top())),
            );

            self::status_label(painter, position, label, color);
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, capture: &GeometryCapture, selected: Option<&Arc<Geometry>>) {
        self.update_scene(capture, ui.ctx());
        self.toolbar(ui, selected);

        let padding = egui::Frame::central_panel(ui.style()).inner_margin.top as f32;
        ui.add_space((padding - ui.spacing().item_spacing.y).max(0.0));

        let size = ui.available_size().max(vec2(1.0, 1.0));
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        self.handle_input(ui, &response);

        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 4.0, Color32::from_rgb(16, 20, 26));

        let camera = Camera::new(self, rect);

        Self::draw_bounds(&painter, &camera);

        let mut surfaces = Vec::new();
        let mut selected_bounds = [egui::Rect::NOTHING; 2];
        let mut hover = None;
        let mut probe: Option<(Vec3, f32)> = None;
        let pointer = response.hover_pos();
        let mut nearest = 6.0_f32;

        for polygon in &self.polygons {
            let draw = &self.cached_draws[polygon.draw];
            let is_selected = selected.is_some_and(|s| Arc::ptr_eq(s, draw));

            let variants: &[(bool, Color32)] = if is_selected {
                &[(false, OWN), (true, FROZEN)]
            } else {
                &[(draw.frozen, SCENE)]
            };

            for &(frozen, color) in variants {
                let hidden = if is_selected {
                    (!frozen && !self.show_own) || (frozen && (!draw.frozen || !self.show_frozen))
                } else {
                    !self.show_scene || (self.writers_only && !(draw.zmode.enable() && draw.zmode.update_enable()))
                };

                if hidden {
                    continue;
                }

                let plane = match (frozen, &draw.reference) {
                    (false, _) => None,
                    (true, Some(reference)) => Some(reference.plane),
                    (true, None) => continue,
                };

                let place = |p: Vec3| plane.map_or(p, |plane| self::on_plane(p, plane));

                let points: Vec<_> = polygon.points.iter().map(|p| place(p.screen)).collect();

                let mut depth = 0.0;
                let xy: Vec<_> = points
                    .iter()
                    .map(|&p| {
                        let (xy, z) = camera.project(p);
                        depth += z;
                        xy
                    })
                    .collect();
                let depth = depth / xy.len() as f32;

                if is_selected {
                    for &p in &xy {
                        selected_bounds[frozen as usize].extend_with(p);
                    }
                }

                let texture = draw
                    .texture
                    .as_ref()
                    .filter(|_| self.show_textures)
                    .and_then(|t| self.textures.get(&(Arc::as_ptr(t) as usize, draw.texture_wrap)))
                    .filter(|_| polygon.points.iter().all(|p| p.uv().is_finite()));

                let (width, fade, opacity, tex_alpha) = if is_selected {
                    (2.0, 1.0, 45, 190)
                } else if texture.is_some() {
                    (0.6, 0.12, 6, 150)
                } else {
                    (0.6, 0.35, 6, 150)
                };

                let stroke = Stroke::new(width, color.gamma_multiply(fade));

                let fill = if texture.is_some() {
                    Color32::from_white_alpha(tex_alpha)
                } else {
                    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), opacity)
                };

                let shapes = self::surface(&xy, fill, stroke, texture.map(|t| (t.id(), polygon.points.as_slice())));
                surfaces.push((depth, shapes));

                let Some(pointer) = pointer else {
                    continue;
                };

                if self.show_test && is_selected && !response.dragged() {
                    for i in 1..xy.len() - 1 {
                        let indices = [0, i, i + 1];

                        let Some(p) = self::probe_point(
                            pointer,
                            indices.map(|j| xy[j]),
                            indices.map(|j| polygon.points[j].screen),
                        ) else {
                            continue;
                        };

                        let z = camera.project(place(p)).1;

                        if probe.is_none_or(|(_, nearest)| z < nearest) {
                            probe = Some((p, z));
                        }
                    }
                }

                for i in 0..xy.len() {
                    let j = (i + 1) % xy.len();
                    let segment = xy[j] - xy[i];
                    let t = ((pointer - xy[i]).dot(segment) / segment.length_sq().max(1e-12)).clamp(0.0, 1.0);

                    let distance = pointer.distance(xy[i] + segment * t);

                    if distance < nearest {
                        nearest = distance;
                        hover = Some((draw, frozen, points[i].lerp(points[j], t)));
                    }
                }
            }
        }

        surfaces.sort_by(|a, b| b.0.total_cmp(&a.0));
        painter.extend(surfaces.into_iter().flat_map(|(_, shapes)| shapes));

        if self.show_plane
            && let Some(draw) = selected
        {
            Self::draw_reference(&painter, &camera, draw);
        }

        Self::draw_bound_labels(&painter, rect, selected_bounds);

        let status = if let Some((point, _)) = probe
            && let Some(draw) = selected
        {
            self::draw_probe(&painter, &camera, draw, point)
        } else if let Some((draw, frozen, p)) = hover {
            format!(
                "{:06X} · {} · depth {:.0}",
                draw.location.offset,
                if frozen { "frozen" } else { "original" },
                p.z,
            )
        } else {
            String::new()
        };

        if !status.is_empty() {
            let label = painter.layout_no_wrap(status, egui::FontId::monospace(12.0), Color32::LIGHT_GRAY);
            let position = rect.left_bottom() + vec2(8.0, -8.0 - label.size().y);

            self::status_label(&painter, position, label, Color32::LIGHT_GRAY);
        }
    }
}
