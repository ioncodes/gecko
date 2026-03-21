use super::regs::{AlphaCompare, BlendMode, MagFilter, MinFilter, TevAlphaEnv, TevColorEnv, WrapMode, ZMode};
use chapa::BitEnum;

#[derive(Debug)]
pub enum Primitive {
    Quads,
    Triangles,
    TriangleStrip,
    TriangleFan,
    Lines,
    LineStrip,
    Points,
}

impl Primitive {
    pub fn from_cmd(cmd: u8) -> Option<Self> {
        use super::constants::*;

        match cmd & !0b111 {
            DRAW_QUADS_CMD => Some(Primitive::Quads),
            DRAW_TRIANGLES_CMD => Some(Primitive::Triangles),
            DRAW_TRIANGLE_STRIP_CMD => Some(Primitive::TriangleStrip),
            DRAW_TRIANGLE_FAN_CMD => Some(Primitive::TriangleFan),
            DRAW_LINES_CMD => Some(Primitive::Lines),
            DRAW_LINE_STRIP_CMD => Some(Primitive::LineStrip),
            DRAW_POINTS_CMD => Some(Primitive::Points),
            _ => {
                tracing::error!(cmd = format!("{:02X}", cmd), "unknown primitive command");
                None
            }
        }
    }
}

#[derive(Debug)]
pub struct Vertex {
    pub position: [f32; 3],
    pub color0: [f32; 4],
    pub texcoords: [Option<[f32; 2]>; 8],
}

#[derive(BitEnum, Debug, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    I4 = 0x0,
    I8 = 0x1,
    IA4 = 0x2,
    IA8 = 0x3,
    RGB565 = 0x4,
    RGB5A3 = 0x5,
    RGBA8 = 0x6,
    CI4 = 0x8,
    CI8 = 0x9,
    CI14 = 0xA,
    CMPR = 0xE,
}

#[derive(Debug, Clone, Copy)]
pub struct TextureDescriptor {
    pub ram_addr: usize,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    pub wrap_s: WrapMode,
    pub wrap_t: WrapMode,
    pub mag_filter: MagFilter,
    pub min_filter: MinFilter,
}

pub struct DrawCall {
    pub primitive: Primitive,
    pub vertices: Vec<Vertex>,
    pub modelview: Matrix4,

    // Textures bound at draw time
    pub textures: [Option<TextureDescriptor>; 8],

    // TEV state snapshot
    pub tev_color_env: [TevColorEnv; 16],
    pub tev_alpha_env: [TevAlphaEnv; 16],
    pub tev_color_regs: [[f32; 4]; 4],
    pub tev_konst_colors: [[f32; 4]; 16],
    pub num_tev_stages: u8,

    // BP state snapshot
    pub bp_zmode: ZMode,
    pub bp_blend_mode: BlendMode,
    pub bp_alpha_compare: AlphaCompare,
}

#[derive(Debug, Clone, Copy)]
pub struct Matrix4(pub [[f32; 4]; 4]);

impl Default for Matrix4 {
    fn default() -> Self {
        Matrix4([[0.0; 4]; 4])
    }
}

impl std::ops::Mul for Matrix4 {
    type Output = Matrix4;

    fn mul(self, rhs: Matrix4) -> Matrix4 {
        let (a, b) = (&self.0, &rhs.0);
        let mut out = [[0.0f32; 4]; 4];
        for col in 0..4 {
            for row in 0..4 {
                out[col][row] =
                    a[0][row] * b[col][0] + a[1][row] * b[col][1] + a[2][row] * b[col][2] + a[3][row] * b[col][3];
            }
        }
        Matrix4(out)
    }
}

#[derive(Default)]
pub struct DrawCommands {
    pub projection: Matrix4,
    pub commands: Vec<DrawCall>,
}
