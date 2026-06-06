use gecko::gekko::condition::ConditionRegister;

#[derive(Clone, Copy)]
pub struct CpuSnapshot {
    pub gprs: [u32; 32],
    pub fprs: [f64; 32],
    pub lr: u32,
    pub ctr: u32,
    pub cr: ConditionRegister,
}

impl CpuSnapshot {
    pub fn from_cpu(cpu: &gecko::gekko::Gekko) -> Self {
        Self {
            gprs: cpu.gprs,
            fprs: cpu.fpr_array(),
            lr: cpu.spr.lr,
            ctr: cpu.spr.ctr,
            cr: cpu.cr,
        }
    }
}
