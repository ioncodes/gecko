#[chapa::bitfield(u32, width = 32, order = msb0)]
#[derive(Clone, Copy, Default)]
pub struct Msr {
    #[bits(13, alias = "pow")]
    pub power_management: bool,

    #[bits(15, alias = "ile")]
    pub exception_little_endian: bool,

    #[bits(16, alias = "ee")]
    pub external_interrupt_enable: bool,

    #[bits(17, alias = "pr")]
    pub privilege_level: bool,

    #[bits(18, alias = "fp")]
    pub floating_point_available: bool,

    #[bits(19, alias = "me")]
    pub machine_check_enable: bool,

    #[bits(20, alias = "fe0")]
    pub floating_point_exception_mode_0: bool,

    #[bits(21, alias = "se")]
    pub single_step_trace: bool,

    #[bits(22, alias = "be")]
    pub branch_trace: bool,

    #[bits(23, alias = "fe1")]
    pub floating_point_exception_mode_1: bool,

    #[bits(25, alias = "ip")]
    pub exception_prefix: bool,

    #[bits(26, alias = "ir")]
    pub instruction_address_translation: bool,

    #[bits(27, alias = "dr")]
    pub data_address_translation: bool,

    #[bits(29, alias = "pm")]
    pub performance_monitor: bool,

    #[bits(30, alias = "ri")]
    pub recoverable_interrupt: bool,

    #[bits(31, alias = "le")]
    pub little_endian: bool,
}
