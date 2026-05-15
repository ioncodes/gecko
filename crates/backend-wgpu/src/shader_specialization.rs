use gecko::flipper::gx::regs::{AlphaCompare, AlphaOp, CompareFunc};
use gecko::host::DrawData;
use std::fs::File;
use std::io::{BufWriter, Read, Write as IoWrite};
use std::path::Path;
use wesl::{VirtualResolver, Wesl};

const COMMON_WESL: &str = include_str!("shaders/common.wesl");
const TEV_HELPERS_WESL: &str = include_str!("shaders/tev_helpers.wesl");
const TEV_COMBINERS_WESL: &str = include_str!("shaders/tev_combiners.wesl");
const TEV_INDIRECT_WESL: &str = include_str!("shaders/tev_indirect.wesl");
const ALPHA_TEST_WESL: &str = include_str!("shaders/alpha_test.wesl");
const LIGHTING_WESL: &str = include_str!("shaders/lighting.wesl");
const MAIN_WESL: &str = include_str!("shaders/main.wesl");

pub(crate) const KEY_BYTES: usize = 6;
const CACHE_MAGIC: [u8; 4] = *b"GSKC";
pub(crate) const CACHE_VERSION: u32 = 6;
pub(crate) const SHADER_CACHE_PATH: &str = "cache/shader_keys.bin";

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub(crate) struct ShaderKey {
    pub num_tev_stages: u8,
    pub num_indirect_stages: u8,
    pub has_lighting_c0: bool,
    pub has_lighting_c1: bool,
    pub alpha_test_enabled: bool,
    pub active_texcoords: u8,
}

impl ShaderKey {
    pub(crate) fn from_draw(draw: &DrawData, alpha_cmp: AlphaCompare) -> Self {
        let num_tev_stages = draw.num_tev_stages.clamp(1, 16);
        let num_indirect_stages = draw.num_indirect_stages.min(4);
        let has_lighting_c0 = draw.color_ctrl[0].enable() || draw.alpha_ctrl[0].enable();
        let has_lighting_c1 = draw.color_ctrl[1].enable() || draw.alpha_ctrl[1].enable();
        let comp0 = alpha_cmp.comp0();
        let comp1 = alpha_cmp.comp1();
        let op = alpha_cmp.op();
        let always_pass =
            comp0 == CompareFunc::Always && comp1 == CompareFunc::Always && matches!(op, AlphaOp::And | AlphaOp::Or);

        Self {
            num_tev_stages,
            num_indirect_stages,
            has_lighting_c0,
            has_lighting_c1,
            alpha_test_enabled: !always_pass,
            active_texcoords: draw.active_texcoords.min(8),
        }
    }
}

fn make_resolver() -> VirtualResolver<'static> {
    let mut r = VirtualResolver::new();
    r.add_module("package::common".parse().unwrap(), COMMON_WESL.into());
    r.add_module("package::tev_helpers".parse().unwrap(), TEV_HELPERS_WESL.into());
    r.add_module("package::tev_combiners".parse().unwrap(), TEV_COMBINERS_WESL.into());
    r.add_module("package::tev_indirect".parse().unwrap(), TEV_INDIRECT_WESL.into());
    r.add_module("package::alpha_test".parse().unwrap(), ALPHA_TEST_WESL.into());
    r.add_module("package::lighting".parse().unwrap(), LIGHTING_WESL.into());
    r.add_module("package::main".parse().unwrap(), MAIN_WESL.into());
    r
}

impl ShaderKey {
    pub(crate) fn to_bytes(&self) -> [u8; KEY_BYTES] {
        [
            self.num_tev_stages,
            self.num_indirect_stages,
            self.has_lighting_c0 as u8,
            self.has_lighting_c1 as u8,
            self.alpha_test_enabled as u8,
            self.active_texcoords,
        ]
    }

    pub(crate) fn from_bytes(b: &[u8; KEY_BYTES]) -> Self {
        Self {
            num_tev_stages: b[0],
            num_indirect_stages: b[1],
            has_lighting_c0: b[2] != 0,
            has_lighting_c1: b[3] != 0,
            alpha_test_enabled: b[4] != 0,
            active_texcoords: b[5].min(8),
        }
    }
}

pub(crate) fn load_cached_keys(path: &Path) -> Vec<ShaderKey> {
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let mut header = [0u8; 8];
    if f.read_exact(&mut header).is_err() {
        return Vec::new();
    }

    if header[..4] != CACHE_MAGIC {
        return Vec::new();
    }

    let version = u32::from_le_bytes(header[4..8].try_into().unwrap());
    if version != CACHE_VERSION {
        return Vec::new();
    }

    let mut keys = Vec::new();
    let mut buf = [0u8; KEY_BYTES];
    while f.read_exact(&mut buf).is_ok() {
        keys.push(ShaderKey::from_bytes(&buf));
    }

    keys
}

pub(crate) fn save_keys(path: &Path, keys: &[ShaderKey]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let f = File::create(path)?;
    let mut w = BufWriter::new(f);
    w.write_all(&CACHE_MAGIC)?;
    w.write_all(&CACHE_VERSION.to_le_bytes())?;
    for k in keys {
        w.write_all(&k.to_bytes())?;
    }
    w.flush()?;
    Ok(())
}

pub(crate) fn compile_variant(key: ShaderKey) -> String {
    let resolver = make_resolver();
    let mut compiler = Wesl::new("").set_custom_resolver(resolver);

    for i in 1..=16u8 {
        compiler.set_feature(&format!("TEV_STAGE_{i}_ENABLED"), i <= key.num_tev_stages);
    }

    for i in 0..4u8 {
        compiler.set_feature(&format!("IND_STAGE_{i}_ENABLED"), i < key.num_indirect_stages);
    }

    compiler.set_feature("HAS_LIGHTING_C0", key.has_lighting_c0);
    compiler.set_feature("HAS_LIGHTING_C1", key.has_lighting_c1);
    compiler.set_feature("ALPHA_TEST_ENABLED", key.alpha_test_enabled);

    for i in 0..8u8 {
        compiler.set_feature(&format!("TEXCOORD_{i}_ENABLED"), i < key.active_texcoords);
    }

    let entry = "package::main".parse().expect("valid module path");
    compiler
        .compile(&entry)
        .expect("WESL specialization failed")
        .to_string()
}
