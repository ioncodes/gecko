use super::GraphicsProcessor;
use super::constants::*;
use super::draw::Primitive;
use glam::{Mat4, Vec3, Vec4};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DepthPlane {
    pub dx: f32,
    pub dy: f32,
    pub c: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ViewportTransform {
    pub scale: Vec3,
    pub offset: Vec3,
}

impl ViewportTransform {
    pub fn to_screen(self, clip: Vec4) -> Vec3 {
        clip.truncate() / clip.w * self.scale + self.offset
    }
}

pub fn clip_distance(clip: Vec4, plane: usize) -> f32 {
    let [x, y, z, w] = clip.to_array();
    [x + w, w - x, y + w, w - y, z + w, -z][plane]
}

impl DepthPlane {
    pub fn from_points(points: [Vec3; 3]) -> Option<Self> {
        let [a, b, c] = points;
        let normal = (b - a).cross(c - a);

        if normal.z == 0.0 {
            return None;
        }

        let dx = -normal.x / normal.z;
        let dy = -normal.y / normal.z;

        let plane = Self {
            dx,
            dy,
            c: a.z - dx * a.x - dy * a.y,
        };

        plane.to_array().iter().all(|v| v.is_finite()).then_some(plane)
    }

    pub fn from_clip_triangle(clip: [Vec4; 3], viewport: ViewportTransform) -> Option<Self> {
        let clipped = (0..6).any(|p| clip.iter().all(|&v| self::clip_distance(v, p) < 0.0));

        if clipped {
            return None;
        }

        Self::from_points(clip.map(|v| viewport.to_screen(v)))
    }

    pub fn last_triangle(
        primitive: Primitive,
        vertex_count: u32,
        viewport: ViewportTransform,
        clip: impl Fn(u32) -> Vec4,
    ) -> Option<([u32; 3], Self)> {
        (0..primitive.triangle_count(vertex_count)).rev().find_map(|i| {
            let triangle = primitive.triangle(i);

            Self::from_clip_triangle(triangle.map(&clip), viewport).map(|plane| (triangle, plane))
        })
    }

    pub fn eval(self, x: f32, y: f32) -> f32 {
        self.dx * x + self.dy * y + self.c
    }

    pub fn to_array(self) -> [f32; 3] {
        [self.dx, self.dy, self.c]
    }
}

impl GraphicsProcessor {
    pub fn viewport_transform(&self) -> ViewportTransform {
        let vp = self.cur_viewport;

        ViewportTransform {
            scale: Vec3::new(
                vp.w * 0.5,
                vp.h * -0.5,
                f32::from_bits(self.xf_mem[XF_VIEWPORT_SCALE_Z]),
            ),
            offset: Vec3::new(
                vp.x + vp.w * 0.5,
                vp.y + vp.h * 0.5,
                f32::from_bits(self.xf_mem[XF_VIEWPORT_OFFSET_Z]),
            ),
        }
    }

    pub fn clip_position(&self, pos_view: [f32; 3]) -> Vec4 {
        Mat4::from_cols_array_2d(&self.projection.0) * Vec3::from_array(pos_view).extend(1.0)
    }

    pub fn effective_ztex_op(&self) -> u8 {
        if self.cur_pe_control.early_ztest() {
            0
        } else {
            super::regs::TevZtex2::from_raw(self.bp_regs[BP_TEV_ZTEX2]).op()
        }
    }

    pub(super) fn update_depth_plane(
        &mut self,
        entries: &[super::vertex::DrawBatchEntry],
        vertices: &[crate::host::DrawVertex],
    ) {
        let viewport = self.viewport_transform();
        let mut end = vertices.len();

        for entry in entries.iter().rev() {
            let start = end - entry.vertex_count;
            end = start;

            let Some(primitive) = Primitive::from_cmd(entry.cmd) else {
                continue;
            };

            let clip = |j: u32| self.clip_position(vertices[start + j as usize].pos_view);

            if let Some((_, plane)) = DepthPlane::last_triangle(primitive, entry.vertex_count as u32, viewport, clip) {
                self.zfreeze_plane = plane;
                return;
            }
        }
    }
}
