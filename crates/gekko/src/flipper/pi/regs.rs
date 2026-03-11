use super::Pi;

// 0xCC003000  4  r    INTSR - Interrupt Cause
crate::mmio_register! {
    InterruptCause: u32 @ 0xCC003000 => Pi.intsr {
        #[bits(0, alias = "error")]
        pub gp_runtime_error: bool,
        
        #[bits(1, alias = "rsw")]
        pub reset_switch: bool,
        
        #[bits(2, alias = "di")]
        pub dvd: bool,
        
        #[bits(3, alias = "si")]
        pub serial: bool,
        
        #[bits(4)]
        pub exi: bool,
        
        #[bits(5, alias = "ai")]
        pub streaming: bool,

        #[bits(6)]
        pub dsp: bool,

        #[bits(7, alias = "mem")]
        pub memory: bool,

        #[bits(8, alias = "vi")]
        pub video: bool,

        #[bits(9, alias = "pe_token")]
        pub token_assertion_in_cmd_list: bool,

        #[bits(10, alias = "pe_finish")]
        pub frame_is_ready: bool,

        #[bits(11, alias = "cp")]
        pub command_fifo: bool,

        #[bits(12)]
        pub debug: bool,

        #[bits(13, alias = "hsp")]
        pub highspeed_port: bool,

        #[bits(16)]
        pub rswst: bool,
    }
}

// 0xCC003004  4  r/w  INTMR - Interrupt Mask
crate::mmio_register! {
    InterruptMask: u32 @ 0xCC003004 => Pi.intmr {
        #[bits(0, alias = "error")]
        pub gp_runtime_error: bool,
        
        #[bits(1, alias = "rsw")]
        pub reset_switch: bool,
        
        #[bits(2, alias = "di")]
        pub dvd: bool,
        
        #[bits(3, alias = "si")]
        pub serial: bool,
        
        #[bits(4)]
        pub exi: bool,
        
        #[bits(5, alias = "ai")]
        pub streaming: bool,

        #[bits(6)]
        pub dsp: bool,

        #[bits(7, alias = "mem")]
        pub memory: bool,

        #[bits(8, alias = "vi")]
        pub video: bool,

        #[bits(9, alias = "pe_token")]
        pub token_assertion_in_cmd_list: bool,

        #[bits(10, alias = "pe_finish")]
        pub frame_is_ready: bool,

        #[bits(11, alias = "cp")]
        pub command_fifo: bool,

        #[bits(12)]
        pub debug: bool,

        #[bits(13, alias = "hsp")]
        pub highspeed_port: bool,
    }
}
