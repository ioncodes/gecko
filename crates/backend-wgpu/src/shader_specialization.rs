use gecko::flipper::gx::regs::{AlphaCompare, AlphaOp, CompareFunc};
use gecko::host::DrawData;
use wesl::{VirtualResolver, Wesl};

const COMMON_WESL: &str = include_str!("shaders/common.wesl");
const TEV_HELPERS_WESL: &str = include_str!("shaders/tev_helpers.wesl");
const TEV_COMBINERS_WESL: &str = include_str!("shaders/tev_combiners.wesl");
const TEV_INDIRECT_WESL: &str = include_str!("shaders/tev_indirect.wesl");
const ALPHA_TEST_WESL: &str = include_str!("shaders/alpha_test.wesl");
const LIGHTING_WESL: &str = include_str!("shaders/lighting.wesl");
const MAIN_WESL: &str = include_str!("shaders/main.wesl");

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub(crate) struct ShaderKey {
    pub num_tev_stages: u8,
    pub num_indirect_stages: u8,
    pub has_lighting: bool,
    pub alpha_test_enabled: bool,
}

impl ShaderKey {
    pub(crate) fn from_draw(draw: &DrawData, alpha_cmp: AlphaCompare) -> Self {
        let num_tev_stages = draw.num_tev_stages.clamp(1, 16);
        let num_indirect_stages = draw.num_indirect_stages.min(4);
        let has_lighting = draw.color_ctrl[0].enable()
            || draw.alpha_ctrl[0].enable()
            || draw.color_ctrl[1].enable()
            || draw.alpha_ctrl[1].enable();
        let comp0 = alpha_cmp.comp0();
        let comp1 = alpha_cmp.comp1();
        let op = alpha_cmp.op();
        let always_pass = comp0 == CompareFunc::Always
            && comp1 == CompareFunc::Always
            && matches!(op, AlphaOp::And | AlphaOp::Or);
        Self {
            num_tev_stages,
            num_indirect_stages,
            has_lighting,
            alpha_test_enabled: !always_pass,
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

pub(crate) fn compile_variant(key: ShaderKey) -> String {
    let mut compiler = Wesl::new("").set_custom_resolver(make_resolver());
    for i in 1..=16u8 {
        compiler.set_feature(&format!("TEV_STAGE_{i}_ENABLED"), i <= key.num_tev_stages);
    }
    for i in 0..4u8 {
        compiler.set_feature(&format!("IND_STAGE_{i}_ENABLED"), i < key.num_indirect_stages);
    }
    compiler.set_feature("HAS_LIGHTING", key.has_lighting);
    compiler.set_feature("ALPHA_TEST_ENABLED", key.alpha_test_enabled);
    let entry = "package::main".parse().expect("valid module path");
    compiler
        .compile(&entry)
        .expect("WESL specialization failed")
        .to_string()
}
