use gecko::flipper::gx::regs::{AlphaCompare, AlphaOp, CompareFunc};
use gecko::host::DrawState;
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
pub(crate) const SPECIALIZATION_KEY_BYTES: usize = 82 * size_of::<u32>();
const CACHE_MAGIC: [u8; 4] = *b"GSKC";
pub(crate) const CACHE_VERSION: u32 = 8;
pub(crate) fn shader_cache_path() -> std::path::PathBuf {
    gecko::paths::cache("shader_keys.bin")
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub(crate) struct ShaderKey {
    pub num_tev_stages: u8,
    pub num_indirect_stages: u8,
    pub has_lighting_c0: bool,
    pub has_lighting_c1: bool,
    pub alpha_test_enabled: bool,
    pub active_texcoords: u8,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug, Default)]
pub(crate) struct ShaderSpecializationKey {
    pub tev_color_env: [u32; 16],
    pub tev_alpha_env: [u32; 16],
    pub tev_orders: [u32; 16],
    pub tev_ksel: [u32; 8],
    pub indirect_refs: u32,
    pub tev_indirect: [u32; 16],
    pub color_ctrl: [u32; 2],
    pub alpha_ctrl: [u32; 2],
    pub alpha_comp0: u32,
    pub alpha_comp1: u32,
    pub alpha_op: u32,
    pub ztex_type: u32,
    pub ztex_op: u32,
}

const TEV_COLOR_ENV_NAMES: [&str; 16] = [
    "gx_tev_color_env_0",
    "gx_tev_color_env_1",
    "gx_tev_color_env_2",
    "gx_tev_color_env_3",
    "gx_tev_color_env_4",
    "gx_tev_color_env_5",
    "gx_tev_color_env_6",
    "gx_tev_color_env_7",
    "gx_tev_color_env_8",
    "gx_tev_color_env_9",
    "gx_tev_color_env_10",
    "gx_tev_color_env_11",
    "gx_tev_color_env_12",
    "gx_tev_color_env_13",
    "gx_tev_color_env_14",
    "gx_tev_color_env_15",
];
const TEV_ALPHA_ENV_NAMES: [&str; 16] = [
    "gx_tev_alpha_env_0",
    "gx_tev_alpha_env_1",
    "gx_tev_alpha_env_2",
    "gx_tev_alpha_env_3",
    "gx_tev_alpha_env_4",
    "gx_tev_alpha_env_5",
    "gx_tev_alpha_env_6",
    "gx_tev_alpha_env_7",
    "gx_tev_alpha_env_8",
    "gx_tev_alpha_env_9",
    "gx_tev_alpha_env_10",
    "gx_tev_alpha_env_11",
    "gx_tev_alpha_env_12",
    "gx_tev_alpha_env_13",
    "gx_tev_alpha_env_14",
    "gx_tev_alpha_env_15",
];
const TEV_ORDER_NAMES: [&str; 16] = [
    "gx_tev_order_0",
    "gx_tev_order_1",
    "gx_tev_order_2",
    "gx_tev_order_3",
    "gx_tev_order_4",
    "gx_tev_order_5",
    "gx_tev_order_6",
    "gx_tev_order_7",
    "gx_tev_order_8",
    "gx_tev_order_9",
    "gx_tev_order_10",
    "gx_tev_order_11",
    "gx_tev_order_12",
    "gx_tev_order_13",
    "gx_tev_order_14",
    "gx_tev_order_15",
];
const TEV_KSEL_NAMES: [&str; 8] = [
    "gx_tev_ksel_0",
    "gx_tev_ksel_1",
    "gx_tev_ksel_2",
    "gx_tev_ksel_3",
    "gx_tev_ksel_4",
    "gx_tev_ksel_5",
    "gx_tev_ksel_6",
    "gx_tev_ksel_7",
];
const TEV_INDIRECT_NAMES: [&str; 16] = [
    "gx_tev_indirect_0",
    "gx_tev_indirect_1",
    "gx_tev_indirect_2",
    "gx_tev_indirect_3",
    "gx_tev_indirect_4",
    "gx_tev_indirect_5",
    "gx_tev_indirect_6",
    "gx_tev_indirect_7",
    "gx_tev_indirect_8",
    "gx_tev_indirect_9",
    "gx_tev_indirect_10",
    "gx_tev_indirect_11",
    "gx_tev_indirect_12",
    "gx_tev_indirect_13",
    "gx_tev_indirect_14",
    "gx_tev_indirect_15",
];

impl ShaderSpecializationKey {
    pub(crate) fn from_draw(draw: &DrawState, alpha_cmp: AlphaCompare, shader: ShaderKey) -> Self {
        let tev_stages = usize::from(shader.num_tev_stages.min(16));
        let indirect_stages = u32::from(shader.num_indirect_stages.min(4));

        let mut key = Self {
            tev_ksel: draw.tev_ksel.map(|value| value & 0x0f),
            indirect_refs: draw.indirect_refs & ((1u32 << (indirect_stages * 6)) - 1),
            color_ctrl: draw.color_ctrl.map(|ctrl| ctrl.raw() & 0x7fff),
            alpha_ctrl: draw.alpha_ctrl.map(|ctrl| ctrl.raw() & 0x7fff),
            ztex_type: u32::from(draw.ztex_type & 3),
            ztex_op: u32::from(draw.ztex_op & 3),
            ..Self::default()
        };

        fn masked_copy(dst: &mut [u32], src: &[u32], mask: u32) {
            for (d, &s) in dst.iter_mut().zip(src) {
                *d = s & mask;
            }
        }

        masked_copy(&mut key.tev_color_env[..tev_stages], &draw.tev_color_env, 0x00ff_ffff);
        masked_copy(&mut key.tev_alpha_env[..tev_stages], &draw.tev_alpha_env, 0x00ff_ffff);
        masked_copy(&mut key.tev_orders[..tev_stages], &draw.tev_orders, 0x03ff);

        if indirect_stages != 0 {
            masked_copy(&mut key.tev_indirect[..tev_stages], &draw.tev_indirect, 0x001f_ffff);
        }

        if shader.alpha_test_enabled {
            key.alpha_comp0 = alpha_cmp.comp0() as u32;
            key.alpha_comp1 = alpha_cmp.comp1() as u32;
            key.alpha_op = alpha_cmp.op() as u32;
        }

        if draw.ztex_op == 0 {
            key.ztex_type = 0;
        }

        key
    }

    pub(crate) fn pipeline_constants(&self, shader: ShaderKey) -> Vec<(&'static str, f64)> {
        let tev_stages = usize::from(shader.num_tev_stages.min(16));

        let mut constants = Vec::with_capacity(83);
        constants.push(("gx_ubershader", 0.0));
        constants.extend(
            TEV_COLOR_ENV_NAMES
                .iter()
                .take(tev_stages)
                .zip(self.tev_color_env)
                .map(|(&name, value)| (name, f64::from(value))),
        );
        constants.extend(
            TEV_ALPHA_ENV_NAMES
                .iter()
                .take(tev_stages)
                .zip(self.tev_alpha_env)
                .map(|(&name, value)| (name, f64::from(value))),
        );
        constants.extend(
            TEV_ORDER_NAMES
                .iter()
                .take(tev_stages)
                .zip(self.tev_orders)
                .map(|(&name, value)| (name, f64::from(value))),
        );
        constants.extend(
            TEV_KSEL_NAMES
                .iter()
                .zip(self.tev_ksel)
                .map(|(&name, value)| (name, f64::from(value))),
        );
        constants.push(("gx_indirect_refs", f64::from(self.indirect_refs)));
        constants.extend(
            TEV_INDIRECT_NAMES
                .iter()
                .take(tev_stages)
                .zip(self.tev_indirect)
                .map(|(&name, value)| (name, f64::from(value))),
        );
        constants.extend([
            ("gx_color_ctrl_0", f64::from(self.color_ctrl[0])),
            ("gx_color_ctrl_1", f64::from(self.color_ctrl[1])),
            ("gx_alpha_ctrl_0", f64::from(self.alpha_ctrl[0])),
            ("gx_alpha_ctrl_1", f64::from(self.alpha_ctrl[1])),
            ("gx_ztex_type", f64::from(self.ztex_type)),
            ("gx_ztex_op", f64::from(self.ztex_op)),
        ]);

        if shader.alpha_test_enabled {
            constants.extend([
                ("gx_alpha_comp_0", f64::from(self.alpha_comp0)),
                ("gx_alpha_comp_1", f64::from(self.alpha_comp1)),
                ("gx_alpha_op", f64::from(self.alpha_op)),
            ]);
        }

        constants
    }

    pub(crate) fn to_bytes(self) -> [u8; SPECIALIZATION_KEY_BYTES] {
        let mut out = [0u8; SPECIALIZATION_KEY_BYTES];
        let mut offset = 0;
        let mut write = |value: u32| {
            out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            offset += 4;
        };
        for values in [
            self.tev_color_env,
            self.tev_alpha_env,
            self.tev_orders,
            self.tev_indirect,
        ] {
            values.into_iter().for_each(&mut write);
        }
        self.tev_ksel.into_iter().for_each(&mut write);
        write(self.indirect_refs);
        self.color_ctrl.into_iter().for_each(&mut write);
        self.alpha_ctrl.into_iter().for_each(&mut write);
        [
            self.alpha_comp0,
            self.alpha_comp1,
            self.alpha_op,
            self.ztex_type,
            self.ztex_op,
        ]
        .into_iter()
        .for_each(&mut write);
        debug_assert_eq!(offset, SPECIALIZATION_KEY_BYTES);
        out
    }

    pub(crate) fn from_bytes(bytes: &[u8; SPECIALIZATION_KEY_BYTES]) -> Self {
        let mut offset = 0;

        let mut read = || {
            let value = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            offset += 4;
            value
        };

        let tev_color_env = std::array::from_fn(|_| read());
        let tev_alpha_env = std::array::from_fn(|_| read());
        let tev_orders = std::array::from_fn(|_| read());
        let tev_indirect = std::array::from_fn(|_| read());
        let tev_ksel = std::array::from_fn(|_| read());
        let indirect_refs = read();
        let color_ctrl = std::array::from_fn(|_| read());
        let alpha_ctrl = std::array::from_fn(|_| read());
        let alpha_comp0 = read();
        let alpha_comp1 = read();
        let alpha_op = read();
        let ztex_type = read();
        let ztex_op = read();

        debug_assert_eq!(offset, SPECIALIZATION_KEY_BYTES);

        Self {
            tev_color_env,
            tev_alpha_env,
            tev_orders,
            tev_ksel,
            indirect_refs,
            tev_indirect,
            color_ctrl,
            alpha_ctrl,
            alpha_comp0,
            alpha_comp1,
            alpha_op,
            ztex_type,
            ztex_op,
        }
    }
}

impl ShaderKey {
    pub(crate) fn from_draw(draw: &DrawState, alpha_cmp: AlphaCompare) -> Self {
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
    let out = compiler
        .compile(&entry)
        .expect("WESL specialization failed")
        .to_string();
    #[cfg(feature = "dump-wgsl")]
    {
        let dir = gecko::paths::cache("wgsl");
        let _ = std::fs::create_dir_all(&dir);
        let name: String = key.to_bytes().iter().map(|b| format!("{b:02X}")).collect();
        let _ = std::fs::write(dir.join(format!("variant_{name}.wgsl")), &out);
    }
    out
}
