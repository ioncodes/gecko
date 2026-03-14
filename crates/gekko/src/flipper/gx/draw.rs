use crate::flipper::gx::constants::DRAW_TRIANGLES_CMD;

#[derive(Debug)]
pub enum Primitive {
    Triangles
}

impl Primitive {
    pub fn from_cmd(cmd: u8) -> Option<Self> {
        match cmd & !0b111 {
            DRAW_TRIANGLES_CMD => Some(Primitive::Triangles),
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
}

pub struct DrawCall {
    pub primitive: Primitive,
    pub vertices: Vec<Vertex>,
}

pub type Matrix4 = [[f32; 4]; 4];

#[derive(Default)]
pub struct DrawCommands {
    pub modelview: Matrix4,         // where is this triangle relative to the camera?
    pub projection: Matrix4,        // how is the 3D scene projected onto the 2D screen?
    pub commands: Vec<DrawCall>,    // the actual draw calls to execute
}