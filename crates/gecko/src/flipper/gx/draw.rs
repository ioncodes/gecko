use super::regs::*;
use chapa::BitEnum;

#[derive(Debug, Clone, Copy, Default)]
pub enum Primitive {
    Quads,
    #[default]
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
    pub color1: [f32; 4],
    pub normal: [f32; 3],
    pub pos_view: [f32; 3],
    pub texcoords: [Option<[f32; 3]>; 8],
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

impl TextureFormat {
    #[inline(always)]
    pub fn is_paletted(self) -> bool {
        matches!(self, TextureFormat::CI4 | TextureFormat::CI8 | TextureFormat::CI14)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum TlutFormat {
    #[default]
    IA8,
    RGB565,
    RGB5A3,
}

impl TlutFormat {
    #[inline(always)]
    pub fn from_raw(val: u32) -> Self {
        match val & 0x3 {
            0 => Self::IA8,
            1 => Self::RGB565,
            2 => Self::RGB5A3,
            _ => {
                tracing::error!(val, "invalid TLUT format");
                Self::IA8
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TlutRef {
    pub tmem_offset: u16,
    pub format: TlutFormat,
}

impl TlutRef {
    #[inline(always)]
    pub fn from_raw(val: u32) -> Self {
        Self {
            tmem_offset: (val & 0x3FF) as u16,
            format: TlutFormat::from_raw(val >> 10),
        }
    }
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

#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub min_depth: f32,
    pub max_depth: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            x: 0.0,
            y: 0.0,
            w: super::constants::EFB_WIDTH as f32,
            h: super::constants::EFB_HEIGHT as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Scissor {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Default for Scissor {
    fn default() -> Self {
        Scissor {
            x: 0,
            y: 0,
            w: super::constants::EFB_WIDTH,
            h: super::constants::EFB_HEIGHT,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Matrix4(pub [[f32; 4]; 4]);

impl Matrix4 {
    /// Flatten the 4x4 column-major matrix into a 16-element array.
    pub fn to_col_array(&self) -> [f32; 16] {
        let m = &self.0;
        [
            m[0][0], m[0][1], m[0][2], m[0][3], m[1][0], m[1][1], m[1][2], m[1][3], m[2][0], m[2][1], m[2][2], m[2][3],
            m[3][0], m[3][1], m[3][2], m[3][3],
        ]
    }
}

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
