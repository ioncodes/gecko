use gecko::flipper::gx::draw::{DrawCall, Primitive};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GpuVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
    pub tex0: [f32; 2],
    pub tex1: [f32; 2],
    pub tex2: [f32; 2],
    pub tex3: [f32; 2],
    pub tex4: [f32; 2],
    pub tex5: [f32; 2],
    pub tex6: [f32; 2],
    pub tex7: [f32; 2],
}

impl From<&gecko::flipper::gx::draw::Vertex> for GpuVertex {
    fn from(v: &gecko::flipper::gx::draw::Vertex) -> Self {
        let tc = |i: usize| v.texcoords[i].unwrap_or([0.0, 0.0]);
        Self {
            position: v.position,
            color: v.color0,
            tex0: tc(0),
            tex1: tc(1),
            tex2: tc(2),
            tex3: tc(3),
            tex4: tc(4),
            tex5: tc(5),
            tex6: tc(6),
            tex7: tc(7),
        }
    }
}

pub(crate) fn triangulate_into(dc: &DrawCall, out: &mut Vec<GpuVertex>) {
    match dc.primitive {
        Primitive::Triangles => {
            out.extend(dc.vertices.iter().map(GpuVertex::from));
        }
        Primitive::Quads => {
            for quad in dc.vertices.chunks(4) {
                if quad.len() < 4 {
                    continue;
                }
                out.push((&quad[0]).into());
                out.push((&quad[1]).into());
                out.push((&quad[2]).into());
                out.push((&quad[0]).into());
                out.push((&quad[2]).into());
                out.push((&quad[3]).into());
            }
        }
        Primitive::TriangleStrip => {
            for i in 2..dc.vertices.len() {
                if i % 2 == 0 {
                    out.push((&dc.vertices[i - 2]).into());
                    out.push((&dc.vertices[i - 1]).into());
                    out.push((&dc.vertices[i]).into());
                } else {
                    out.push((&dc.vertices[i - 1]).into());
                    out.push((&dc.vertices[i - 2]).into());
                    out.push((&dc.vertices[i]).into());
                }
            }
        }
        Primitive::TriangleFan => {
            for i in 2..dc.vertices.len() {
                out.push((&dc.vertices[0]).into());
                out.push((&dc.vertices[i - 1]).into());
                out.push((&dc.vertices[i]).into());
            }
        }
        _ => unimplemented!("triangulation for {:?}", dc.primitive),
    }
}

pub(crate) fn align_up(value: u64, alignment: u64) -> u64 {
    (value + alignment - 1) & !(alignment - 1)
}
