use gecko::flipper::gx::GraphicsProcessor;
use gecko::flipper::gx::constants::*;
use gecko::flipper::gx::depth::{self, DepthPlane, ViewportTransform};
use gecko::flipper::gx::draw::Primitive;
use gecko::flipper::gx::regs::{GenMode, WrapMode, ZMode};
use gecko::host::{DrawVertex, GxAction, TextureKey};
use glam::{Vec3, Vec4};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawLocation {
    pub frame: usize,
    pub row: usize,
    pub offset: usize,
}

#[derive(Clone, Debug)]
pub struct Vertex {
    pub clip: Vec4,
    pub stq: Vec3,
}

#[derive(Clone, Copy)]
pub struct ProjectedVertex {
    pub screen: Vec3,
    pub stq: Vec3,
}

impl ProjectedVertex {
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            screen: self.screen.lerp(other.screen, t),
            stq: self.stq.lerp(other.stq, t),
        }
    }

    pub fn uv(self) -> egui::Pos2 {
        let uv = self.stq.truncate() / self.stq.z;

        egui::pos2(uv.x, uv.y)
    }
}

#[derive(Clone, Debug)]
pub struct Reference {
    pub draw: Arc<Geometry>,
    pub triangle: [usize; 3],
    pub plane: DepthPlane,
}

#[derive(Debug)]
pub struct Geometry {
    pub location: DrawLocation,
    pub subdraw: usize,
    pub vertices: Vec<Vertex>,
    pub triangles: Vec<[usize; 3]>,
    pub viewport: ViewportTransform,
    pub frozen: bool,
    pub zmode: ZMode,
    pub z_texture: bool,
    pub depth_before: OnceLock<Option<Vec<u32>>>,
    pub reference: Option<Reference>,
    pub last_plane: Option<([usize; 3], DepthPlane)>,
    pub texture: Option<Arc<egui::ColorImage>>,
    pub texture_wrap: egui::TextureWrapMode,
}

#[derive(Default)]
pub struct GeometryCapture {
    pub location: DrawLocation,
    pub draws: HashMap<usize, Vec<Arc<Geometry>>>,
    pub reference: Option<Reference>,
    textures: HashMap<TextureKey, Arc<egui::ColorImage>>,
    bindings: [Option<TextureKey>; 8],
}

impl GeometryCapture {
    pub fn action(&mut self, action: &GxAction) {
        match action {
            GxAction::LoadTexture {
                id,
                width,
                height,
                rgba,
                ..
            } => {
                let size = [*width as usize, *height as usize];

                if !rgba.is_empty() && rgba.len() >= size[0] * size[1] * 4 {
                    let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba[..size[0] * size[1] * 4]);
                    self.textures.insert(*id, Arc::new(image));
                } else {
                    self.textures.remove(id);
                }
            }
            GxAction::SetTexture { slot, id, .. } => self.bindings[*slot] = Some(*id),
            GxAction::InvalidateCaches => {
                self.textures.clear();
                self.bindings.fill(None);
            }

            _ => {}
        }
    }

    pub fn record(&mut self, gx: &GraphicsProcessor, primitive: Primitive, verts: &[DrawVertex]) {
        let gen_mode = GenMode::from_raw(gx.bp_regs[BP_GEN_MODE]);
        let stages = gen_mode.num_tev_stages() as usize + 1;

        let orders = gx.resolve_tev_orders();
        let texture_stage = orders[..stages]
            .iter()
            .find(|order| order.tex_enable())
            .map(|order| (order.texmap() as usize, order.texcoord() as usize));

        let texture = texture_stage.and_then(|(slot, _)| self.textures.get(&self.bindings[slot]?).cloned());

        let texture_wrap = match texture_stage.and_then(|(slot, _)| gx.cur_textures[slot]) {
            Some(desc) if desc.wrap_s == WrapMode::Repeat => egui::TextureWrapMode::Repeat,
            Some(desc) if desc.wrap_s == WrapMode::Mirror => egui::TextureWrapMode::MirroredRepeat,
            _ => egui::TextureWrapMode::ClampToEdge,
        };

        let vertices: Vec<_> = verts
            .iter()
            .map(|v| {
                let mut stq = texture_stage.map_or(Vec3::Z, |(_, coord)| Vec3::from_array(v.texcoords[coord]));

                if stq.z.abs() < f32::EPSILON {
                    stq.z = 1.0;
                }

                Vertex {
                    clip: gx.clip_position(v.pos_view),
                    stq,
                }
            })
            .collect();

        let viewport = gx.viewport_transform();
        let vertex_count = verts.len() as u32;

        let triangles: Vec<_> = primitive
            .triangles(vertex_count)
            .map(|t| t.map(|i| i as usize))
            .collect();

        let last_plane = DepthPlane::last_triangle(primitive, vertex_count, viewport, |i| vertices[i as usize].clip)
            .map(|(t, plane)| (t.map(|i| i as usize), plane));

        let frozen = gen_mode.z_freeze();
        let draws = self.draws.entry(self.location.offset).or_default();

        let geometry = Arc::new(Geometry {
            location: self.location,
            subdraw: draws.len(),
            vertices,
            triangles,
            viewport,
            frozen,
            zmode: gx.cur_zmode,
            z_texture: gx.effective_ztex_op() != 0,
            depth_before: OnceLock::new(),
            reference: frozen.then(|| self.reference.clone()).flatten(),
            last_plane,
            texture,
            texture_wrap,
        });

        if !frozen && let Some((triangle, plane)) = last_plane {
            self.reference = Some(Reference {
                draw: geometry.clone(),
                triangle,
                plane,
            });
        }

        draws.push(geometry);
    }
}

pub(super) fn clip_polygon<T: Copy, P: Copy>(
    mut polygon: Vec<T>,
    planes: impl IntoIterator<Item = P>,
    distance: impl Fn(T, P) -> f32,
    lerp: impl Fn(T, T, f32) -> T,
) -> Vec<T> {
    let mut input = Vec::with_capacity(polygon.len() + 1);

    for plane in planes {
        std::mem::swap(&mut input, &mut polygon);
        polygon.clear();

        for i in 0..input.len() {
            let a = input[i];
            let b = input[(i + 1) % input.len()];

            let da = distance(a, plane);
            let db = distance(b, plane);

            if da >= 0.0 {
                polygon.push(a);
            }

            if (da < 0.0) != (db < 0.0) {
                polygon.push(lerp(a, b, da / (da - db)));
            }
        }

        if polygon.is_empty() {
            break;
        }
    }

    polygon
}

impl Geometry {
    pub fn projected_vertices(&self, triangle: [usize; 3]) -> Vec<ProjectedVertex> {
        let polygon: Vec<_> = triangle.map(|i| (self.vertices[i].clip, self.vertices[i].stq)).into();

        if polygon.iter().any(|(p, _)| !p.is_finite()) {
            return Vec::new();
        }

        let polygon = self::clip_polygon(
            polygon,
            0..6,
            |(p, _), plane| depth::clip_distance(p, plane),
            |a, b, t| (a.0.lerp(b.0, t), a.1.lerp(b.1, t)),
        );

        polygon
            .into_iter()
            .filter(|(p, _)| p.w > 0.0)
            .map(|(p, stq)| ProjectedVertex {
                screen: self.viewport.to_screen(p),
                stq: stq / p.w,
            })
            .collect()
    }
}
