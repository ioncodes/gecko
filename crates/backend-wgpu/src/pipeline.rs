use crate::helpers;
use crate::shader_specialization::{
    KEY_BYTES as SHADER_KEY_BYTES, SPECIALIZATION_KEY_BYTES, ShaderKey, ShaderSpecializationKey,
};
use chapa::BitField;
use gecko::flipper::gx::regs::{BlendFactor, CompareFunc, CullMode, LogicOp};

fn make_blend_state(
    src_factor: wgpu::BlendFactor,
    dst_factor: wgpu::BlendFactor,
    operation: wgpu::BlendOperation,
) -> wgpu::BlendState {
    let component = wgpu::BlendComponent {
        src_factor,
        dst_factor,
        operation,
    };
    wgpu::BlendState {
        color: component,
        alpha: component,
    }
}

// Blend factor approximations for logic ops.
fn logic_op_approximation(op: LogicOp) -> (bool, bool, wgpu::BlendFactor, wgpu::BlendFactor) {
    use wgpu::BlendFactor::*;
    match op {
        LogicOp::Clear => (true, false, Zero, Zero),
        LogicOp::And => (true, false, Dst, Zero),
        LogicOp::ReverseAnd => (true, true, One, OneMinusSrc),
        LogicOp::Copy => (false, false, One, Zero),
        LogicOp::InvertedAnd => (true, true, Dst, One),
        LogicOp::Noop => (true, false, Zero, One),
        LogicOp::Xor => (true, false, OneMinusDst, OneMinusSrc),
        LogicOp::Or => (true, false, OneMinusDst, One),
        LogicOp::Nor => (true, false, OneMinusDst, OneMinusSrc),
        LogicOp::Equivalent => (true, false, OneMinusDst, Zero),
        LogicOp::Invert => (true, false, OneMinusDst, Zero),
        LogicOp::ReverseOr => (true, false, One, OneMinusDstAlpha),
        LogicOp::InvertedCopy => (false, false, One, Zero),
        LogicOp::InvertedOr => (true, false, OneMinusDst, One),
        LogicOp::Nand => (true, false, OneMinusDst, OneMinusSrc),
        LogicOp::Set => (false, false, One, Zero),
    }
}
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) struct PipelineKey {
    pub blend_enable: bool,
    pub src_factor: BlendFactor,
    pub dst_factor: BlendFactor,
    pub subtract: bool,
    pub logic_op_enable: bool,
    pub logic_op: LogicOp,
    pub z_enable: bool,
    pub z_func: CompareFunc,
    pub z_write: bool,
    pub color_update: bool,
    pub alpha_update: bool,
    pub cull_mode: CullMode,
    /// Z-texturing active for this draw: selects the `fs_main_ztex` entry
    /// point, which writes `frag_depth` (disabling early-Z only where needed).
    pub ztex: bool,
}

impl PipelineKey {
    const BYTES: usize = 13;

    fn to_bytes(self) -> [u8; Self::BYTES] {
        [
            self.blend_enable as u8,
            self.src_factor.raw(),
            self.dst_factor.raw(),
            self.subtract as u8,
            self.logic_op_enable as u8,
            self.logic_op.raw(),
            self.z_enable as u8,
            self.z_func.raw(),
            self.z_write as u8,
            self.color_update as u8,
            self.alpha_update as u8,
            self.cull_mode.raw(),
            self.ztex as u8,
        ]
    }

    fn from_bytes(b: &[u8; Self::BYTES]) -> Self {
        Self {
            blend_enable: b[0] != 0,
            src_factor: BlendFactor::from_raw(b[1]),
            dst_factor: BlendFactor::from_raw(b[2]),
            subtract: b[3] != 0,
            logic_op_enable: b[4] != 0,
            logic_op: LogicOp::from_raw(b[5]),
            z_enable: b[6] != 0,
            z_func: CompareFunc::from_raw(b[7]),
            z_write: b[8] != 0,
            color_update: b[9] != 0,
            alpha_update: b[10] != 0,
            cull_mode: CullMode::from_raw(b[11]),
            ztex: b[12] != 0,
        }
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) struct FullPipelineKey {
    pub shader: ShaderKey,
    pub specialization: ShaderSpecializationKey,
    pub fixed: PipelineKey,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) struct UberPipelineKey {
    pub shader: ShaderKey,
    pub fixed: PipelineKey,
}

impl From<FullPipelineKey> for UberPipelineKey {
    fn from(key: FullPipelineKey) -> Self {
        Self {
            shader: key.shader,
            fixed: key.fixed,
        }
    }
}

pub(crate) const FULL_PIPELINE_KEY_BYTES: usize = SHADER_KEY_BYTES + SPECIALIZATION_KEY_BYTES + PipelineKey::BYTES;
const PIPELINE_CACHE_MAGIC: [u8; 4] = *b"GPKC";
const PIPELINE_CACHE_VERSION: u32 = crate::shader_specialization::CACHE_VERSION;
pub(crate) const PIPELINE_CACHE_PATH: &str = "cache/pipeline_keys.bin";

impl FullPipelineKey {
    fn to_bytes(self) -> [u8; FULL_PIPELINE_KEY_BYTES] {
        let mut out = [0u8; FULL_PIPELINE_KEY_BYTES];
        out[..SHADER_KEY_BYTES].copy_from_slice(&self.shader.to_bytes());
        out[SHADER_KEY_BYTES..SHADER_KEY_BYTES + SPECIALIZATION_KEY_BYTES]
            .copy_from_slice(&self.specialization.to_bytes());
        out[SHADER_KEY_BYTES + SPECIALIZATION_KEY_BYTES..].copy_from_slice(&self.fixed.to_bytes());
        out
    }

    fn from_bytes(b: &[u8; FULL_PIPELINE_KEY_BYTES]) -> Self {
        let mut shader_bytes = [0u8; SHADER_KEY_BYTES];
        shader_bytes.copy_from_slice(&b[..SHADER_KEY_BYTES]);

        let mut specialization_bytes = [0u8; SPECIALIZATION_KEY_BYTES];
        specialization_bytes.copy_from_slice(&b[SHADER_KEY_BYTES..SHADER_KEY_BYTES + SPECIALIZATION_KEY_BYTES]);

        let mut fixed_bytes = [0u8; PipelineKey::BYTES];
        fixed_bytes.copy_from_slice(&b[SHADER_KEY_BYTES + SPECIALIZATION_KEY_BYTES..]);

        Self {
            shader: ShaderKey::from_bytes(&shader_bytes),
            specialization: ShaderSpecializationKey::from_bytes(&specialization_bytes),
            fixed: PipelineKey::from_bytes(&fixed_bytes),
        }
    }
}

pub(crate) fn load_cached_pipeline_keys(path: &Path) -> Vec<FullPipelineKey> {
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let mut header = [0u8; 8];
    if f.read_exact(&mut header).is_err() {
        return Vec::new();
    }

    if header[..4] != PIPELINE_CACHE_MAGIC {
        return Vec::new();
    }

    let version = u32::from_le_bytes(header[4..8].try_into().unwrap());
    if version != PIPELINE_CACHE_VERSION {
        return Vec::new();
    }

    let mut keys = Vec::new();
    let mut buf = [0u8; FULL_PIPELINE_KEY_BYTES];
    while f.read_exact(&mut buf).is_ok() {
        keys.push(FullPipelineKey::from_bytes(&buf));
    }

    keys
}

pub(crate) fn save_pipeline_keys(path: &Path, keys: &[FullPipelineKey]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let f = File::create(path)?;

    let mut w = BufWriter::new(f);
    w.write_all(&PIPELINE_CACHE_MAGIC)?;
    w.write_all(&PIPELINE_CACHE_VERSION.to_le_bytes())?;

    for k in keys {
        w.write_all(&k.to_bytes())?;
    }

    w.flush()?;

    Ok(())
}

pub(crate) fn create_specialized_pipeline(
    device: &wgpu::Device,
    pipeline_layout: &wgpu::PipelineLayout,
    surface_format: wgpu::TextureFormat,
    shader: &wgpu::ShaderModule,
    full_key: &FullPipelineKey,
) -> wgpu::RenderPipeline {
    let constants = full_key.specialization.pipeline_constants(full_key.shader);
    create_pipeline(
        device,
        pipeline_layout,
        surface_format,
        shader,
        full_key.shader,
        &full_key.fixed,
        &constants,
        "gx_pipeline_specialized",
    )
}

pub(crate) fn create_uber_pipeline(
    device: &wgpu::Device,
    pipeline_layout: &wgpu::PipelineLayout,
    surface_format: wgpu::TextureFormat,
    shader: &wgpu::ShaderModule,
    key: &UberPipelineKey,
) -> wgpu::RenderPipeline {
    create_pipeline(
        device,
        pipeline_layout,
        surface_format,
        shader,
        key.shader,
        &key.fixed,
        &[("gx_ubershader", 1.0)],
        "gx_pipeline_uber",
    )
}

#[allow(clippy::too_many_arguments)]
fn create_pipeline(
    device: &wgpu::Device,
    pipeline_layout: &wgpu::PipelineLayout,
    surface_format: wgpu::TextureFormat,
    shader: &wgpu::ShaderModule,
    shader_key: ShaderKey,
    key: &PipelineKey,
    constants: &[(&str, f64)],
    label: &str,
) -> wgpu::RenderPipeline {
    let active_texcoords = shader_key.active_texcoords.min(8) as usize;

    const BASE_ATTRS: [wgpu::VertexAttribute; 5] = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: 12,
            shader_location: 1,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: 28,
            shader_location: 2,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 44,
            shader_location: 3,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 56,
            shader_location: 4,
        },
    ];

    let mut attrs: Vec<wgpu::VertexAttribute> = Vec::with_capacity(5 + active_texcoords);
    attrs.extend_from_slice(&BASE_ATTRS);

    for i in 0..active_texcoords {
        attrs.push(wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 68 + (i as u64) * 12,
            shader_location: 5 + i as u32,
        });
    }

    let vertex_layout = wgpu::VertexBufferLayout {
        array_stride: 68 + (active_texcoords as u64) * 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &attrs,
    };

    let blend = if key.blend_enable {
        // In subtract mode the hardware ignores the configured src/dst
        // factors and computes `dst = dst - src` with ONE/ONE.
        let (src_factor, dst_factor, operation) = if key.subtract {
            (
                wgpu::BlendFactor::One,
                wgpu::BlendFactor::One,
                wgpu::BlendOperation::ReverseSubtract,
            )
        } else {
            (
                helpers::map_src_blend_factor(key.src_factor),
                helpers::map_dst_blend_factor(key.dst_factor),
                wgpu::BlendOperation::Add,
            )
        };
        Some(make_blend_state(src_factor, dst_factor, operation))
    } else if key.logic_op_enable {
        let (be, sub, sf, df) = logic_op_approximation(key.logic_op);
        be.then(|| {
            let operation = if sub {
                wgpu::BlendOperation::ReverseSubtract
            } else {
                wgpu::BlendOperation::Add
            };
            make_blend_state(sf, df, operation)
        })
    } else {
        None
    };

    let depth_stencil = if key.z_enable {
        Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: Some(key.z_write),
            depth_compare: Some(helpers::map_compare_func(key.z_func)),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        })
    } else {
        Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth24Plus,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        })
    };

    let mut write_mask = wgpu::ColorWrites::empty();
    if key.color_update {
        write_mask |= wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN | wgpu::ColorWrites::BLUE;
    }
    if key.alpha_update {
        write_mask |= wgpu::ColorWrites::ALPHA;
    }

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[vertex_layout],
            compilation_options: wgpu::PipelineCompilationOptions {
                constants,
                ..Default::default()
            },
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(if key.ztex { "fs_main_ztex" } else { "fs_main" }),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend,
                write_mask,
            })],
            compilation_options: wgpu::PipelineCompilationOptions {
                constants,
                ..Default::default()
            },
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Cw,
            cull_mode: match key.cull_mode {
                CullMode::Back => Some(wgpu::Face::Back),
                CullMode::Front => Some(wgpu::Face::Front),
                CullMode::None | CullMode::All => None,
            },
            ..Default::default()
        },
        depth_stencil,
        multisample: wgpu::MultisampleState {
            count: crate::EFB_SAMPLE_COUNT,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct PipelineCompiler {
    request_tx: crossbeam_channel::Sender<PipelineCompileRequest>,
    result_rx: crossbeam_channel::Receiver<PipelineCompileResult>,
    shutdown_tx: crossbeam_channel::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
struct PipelineCompileRequest {
    generation: u64,
    key: FullPipelineKey,
    shader: wgpu::ShaderModule,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct PipelineCompileResult {
    pub generation: u64,
    pub key: FullPipelineKey,
    pub pipeline: wgpu::RenderPipeline,
}

#[cfg(not(target_arch = "wasm32"))]
impl PipelineCompiler {
    const QUEUE_LIMIT: usize = 64;

    pub(crate) fn new(
        device: wgpu::Device,
        pipeline_layout: wgpu::PipelineLayout,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let (request_tx, request_rx) = crossbeam_channel::bounded::<PipelineCompileRequest>(Self::QUEUE_LIMIT);
        let (result_tx, result_rx) = crossbeam_channel::unbounded::<PipelineCompileResult>();
        let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded::<()>(1);
        let thread = std::thread::Builder::new()
            .name("gx pipeline compiler".into())
            .spawn(move || {
                loop {
                    crossbeam_channel::select_biased! {
                        recv(shutdown_rx) -> _ => break,
                        recv(request_rx) -> request => {
                            let Ok(request) = request else { break };
                            let pipeline = create_specialized_pipeline(
                                &device,
                                &pipeline_layout,
                                surface_format,
                                &request.shader,
                                &request.key,
                            );
                            let result = PipelineCompileResult {
                                generation: request.generation,
                                key: request.key,
                                pipeline,
                            };
                            if result_tx.send(result).is_err() {
                                break;
                            }
                        }
                    }
                }
            })
            .expect("failed to start GX pipeline compiler");

        Self {
            request_tx,
            result_rx,
            shutdown_tx,
            thread: Some(thread),
        }
    }

    pub(crate) fn request(&self, generation: u64, key: FullPipelineKey, shader: wgpu::ShaderModule) -> bool {
        self.request_tx
            .try_send(PipelineCompileRequest {
                generation,
                key,
                shader,
            })
            .is_ok()
    }

    pub(crate) fn try_recv(&self) -> Option<PipelineCompileResult> {
        self.result_rx.try_recv().ok()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for PipelineCompiler {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.try_send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
