use crate::flipper::vi;
use crate::mmio::traits::{MmioAccess, WriteMask};
use crate::system::{System, SystemId};
use chapa::BitEnum;

// 0xCC002000	2	R/W	VTR (Vertical Timing Register)

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct VerticalTiming {
    #[bits(0..=3, alias = "equ")]
    pub equalization_pulse: u8,
    #[bits(4..=13, alias = "acv")]
    pub active_video: u16,
}
crate::mmio_reg!(VerticalTiming: u16 @ 0xCC002000);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for VerticalTiming {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.vi.vtr
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.vi.vtr = self;
        vi::ensure_half_line_scheduled(sys);
    }
}

// 0xCC002002	2	R/W	DCR (Display Configuration Register)

pub enum RefreshRate {
    Hz60,
    Hz50,
}

impl RefreshRate {
    #[inline(always)]
    pub const fn cycles_per_frame(&self, system: SystemId) -> u64 {
        let clock = crate::scheduler::cpu_clock(system);
        match self {
            RefreshRate::Hz60 => clock / 60,
            RefreshRate::Hz50 => clock / 50,
        }
    }
}

#[derive(Debug, BitEnum, PartialEq, Eq)]
pub enum VideoFormat {
    Ntsc = 0,
    Pal = 1,
    Mpal = 2,
    Debug = 3,
}

impl VideoFormat {
    pub fn refresh_rate(&self) -> RefreshRate {
        match self {
            VideoFormat::Ntsc | VideoFormat::Mpal => RefreshRate::Hz60,
            VideoFormat::Pal | VideoFormat::Debug => RefreshRate::Hz50,
        }
    }

    pub fn lines(&self) -> usize {
        match self {
            VideoFormat::Ntsc | VideoFormat::Mpal => 480,
            VideoFormat::Pal | VideoFormat::Debug => 574,
        }
    }

    pub fn columns(&self) -> usize {
        640
    }
}

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DisplayConfiguration {
    #[bits(0, alias = "enb")]
    pub enable: bool,
    #[bits(1, alias = "rst")]
    pub reset: bool,
    #[bits(2, alias = "nin")]
    pub interlace_selector: bool,
    #[bits(3, alias = "dlr")]
    pub display_mode_3d: bool,
    #[bits(4..=5, alias = "le0")]
    pub display_latch0: u8,
    #[bits(6..=7, alias = "le1")]
    pub display_latch1: u8,
    #[bits(8..=9, alias = "fmt")]
    pub video_format: VideoFormat,
}
crate::mmio_reg!(DisplayConfiguration: u16 @ 0xCC002002);

impl DisplayConfiguration {
    pub fn interlaced(&self) -> bool {
        !self.nin()
    }
}

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DisplayConfiguration {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.vi.dcr
    }

    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        // TODO: Rising edge on RST clears the register. Just a test for now.
        if self.reset() && !sys.vi.dcr.reset() {
            sys.vi.dcr = DisplayConfiguration::from_raw(0);
        } else {
            sys.vi.dcr = self;
        }

        vi::ensure_half_line_scheduled(sys);
    }
}

// 0xCC002004	4	R/W	HTR0 (Horizontal Timing 0)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct HorizontalTiming0 {
    #[bits(0..=8, alias = "hlw")]
    pub halfline_width: u16,
    #[bits(16..=22, alias = "hce")]
    pub horizontal_sync_end: u8,
    #[bits(24..=30, alias = "hcs")]
    pub horizontal_sync_start: u8,
}
crate::mmio_reg!(HorizontalTiming0: u32 @ 0xCC002004);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for HorizontalTiming0 {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.vi.htr0
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.vi.htr0 = self;
        vi::ensure_half_line_scheduled(sys);
    }
}

// 0xCC002008	4	R/W	HTR1 (Horizontal Timing 1)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct HorizontalTiming1 {
    #[bits(0..=6, alias = "hsy")]
    pub horizontal_sync_width: u8,
    #[bits(7..=16, alias = "hbe")]
    pub horizontal_blank_end: u16,
    #[bits(17..=26, alias = "hbs")]
    pub horizontal_blank_start: u16,
}
crate::mmio_reg!(HorizontalTiming1: u32 @ 0xCC002008);
crate::mmio_default_access!(HorizontalTiming1 => System.vi.htr1);

// 0xCC00200C	4	R/W	VTO (Odd Field Vertical Timing Register)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct VerticalTimingOdd {
    #[bits(0..=9, alias = "prb")]
    pub pre_blanking_in_half_lines: u16,
    #[bits(16..=25, alias = "psb")]
    pub post_blanking_in_half_lines: u16,
}
crate::mmio_reg!(VerticalTimingOdd: u32 @ 0xCC00200C);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for VerticalTimingOdd {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.vi.vto
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.vi.vto = self;
        vi::ensure_half_line_scheduled(sys);
    }
}

// 0xCC002010	4	R/W	VTE (Even Field Vertical Timing Register)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct VerticalTimingEven {
    #[bits(0..=9, alias = "prb")]
    pub pre_blanking_in_half_lines: u16,
    #[bits(16..=25, alias = "psb")]
    pub post_blanking_in_half_lines: u16,
}
crate::mmio_reg!(VerticalTimingEven: u32 @ 0xCC002010);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for VerticalTimingEven {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.vi.vte
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.vi.vte = self;
        vi::ensure_half_line_scheduled(sys);
    }
}

// 0xCC002014	4	R/W	BBEI (Burst Blanking Even Interval)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct BurstBlankingEvenInterval {
    #[bits(0..=4, alias = "bs1")]
    pub burst_start_1: u8,
    #[bits(5..=15, alias = "be1")]
    pub burst_end_1: u16,
    #[bits(16..=20, alias = "bs3")]
    pub burst_start_3: u8,
    #[bits(21..=31, alias = "be3")]
    pub burst_end_3: u16,
}
crate::mmio_reg!(BurstBlankingEvenInterval: u32 @ 0xCC002014);
crate::mmio_default_access!(BurstBlankingEvenInterval => System.vi.bbei);

// 0xCC002018	4	R/W	BBOI (Burst Blanking Odd Interval)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct BurstBlankingOddInterval {
    #[bits(0..=4, alias = "bs2")]
    pub burst_start_2: u8,
    #[bits(5..=15, alias = "be2")]
    pub burst_end_2: u16,
    #[bits(16..=20, alias = "bs4")]
    pub burst_start_4: u8,
    #[bits(21..=31, alias = "be4")]
    pub burst_end_4: u16,
}
crate::mmio_reg!(BurstBlankingOddInterval: u32 @ 0xCC002018);
crate::mmio_default_access!(BurstBlankingOddInterval => System.vi.bboi);

// 0xCC00201c	4	R/W	TFBL (Top Field Base Register L, External Framebuffer Half 1)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct TopFieldBase {
    #[bits(0..=23, alias = "fbb")]
    pub xfb_addr: u32,

    #[bits(24..=27, alias = "xof")]
    pub horizontal_offset: u8,

    #[bits(28)]
    pub page_offset: bool,
    // TODO: 29..=31 y always zero (maybe some write only control register stuff, setting bit 31 clears bits 31..=28).
}
crate::mmio_reg!(TopFieldBase: u32 @ 0xCC00201C);
crate::mmio_default_access!(TopFieldBase => System.vi.tfbl);

// 0xCC002020	4	R/W	TFBR (Top Field Base Register R)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct TopFieldBaseRight {
    #[bits(0..=23, alias = "fbb")]
    pub xfb_addr: u32,
}
crate::mmio_reg!(TopFieldBaseRight: u32 @ 0xCC002020);
crate::mmio_default_access!(TopFieldBaseRight => System.vi.tfbr);

// 0xCC002024	4	R/W	BFBL (Bottom Field Base Register L, External Framebuffer Half 2)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct BottomFieldBase {
    #[bits(0..=23, alias = "fbb")]
    pub xfb_addr: u32,

    #[bits(28)]
    pub page_offset: bool,
}
crate::mmio_reg!(BottomFieldBase: u32 @ 0xCC002024);
crate::mmio_default_access!(BottomFieldBase => System.vi.bfbl);

// 0xCC002028	4	R/W	BFBR (Bottom Field Base Register R)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct BottomFieldBaseRight {
    #[bits(0..=23, alias = "fbb")]
    pub xfb_addr: u32,
}
crate::mmio_reg!(BottomFieldBaseRight: u32 @ 0xCC002028);
crate::mmio_default_access!(BottomFieldBaseRight => System.vi.bfbr);

// 0xCC00202C	2	R	DPV (Display Position Vertical)

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DisplayPositionVertical {
    #[bits(0..=10, alias = "vct")]
    pub vertical_count: u16,
}
crate::mmio_reg!(DisplayPositionVertical: u16 @ 0xCC00202C);
crate::mmio_default_access!(DisplayPositionVertical => System.vi.dpv);

// 0xCC00202E	2	R	DPH (Display Position Horizontal)
//
// Read returns a live value computed from the current cycle delta inside the
// half line. Writes are ignored.

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DisplayPositionHorizontal {
    #[bits(0..=10, alias = "hct")]
    pub horizontal_count: u16,
}
crate::mmio_reg!(DisplayPositionHorizontal: u16 @ 0xCC00202E);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DisplayPositionHorizontal {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        let cycles = sys.scheduler.cycles;
        DisplayPositionHorizontal::from_raw(sys.vi.dph_value(SYSTEM, cycles))
    }

    fn write(self, _gc: &mut System<SYSTEM>, _: WriteMask) {
        // Read only.
    }
}

// 0xCC00202C  4  R  DPV/DPH combined access slot. Exists to service u32 reads
// that span both 16 bit position registers; neither one individually fits a
// u32 access at 0x2C, so we add a dedicated combined slot at u32 width. Writes
// land here on full u32 stores, which are ignored because both underlying
// regs are read only.

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DisplayPositionCombined {}
crate::mmio_reg!(DisplayPositionCombined: u32 @ 0xCC00202C);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DisplayPositionCombined {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        let dpv = sys.vi.dpv.raw() as u32;
        let dph = sys.vi.dph_value(SYSTEM, sys.scheduler.cycles) as u32;
        DisplayPositionCombined::from_raw((dpv << 16) | dph)
    }

    fn write(self, _gc: &mut System<SYSTEM>, _: WriteMask) {
        // Read only.
    }
}

// 0xCC002030	4	R/W	DI0 (Display Interrupt 0)
// NOT write 1 to clear. The interrupt bit is a normal R/W flag.
//
// DI0..=DI3 each trigger a PI interrupt when their enable + interrupt bits
// are both set. Writes to these registers must refresh the VI->PI line.

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DisplayInterrupt0 {
    #[bits(0..=9, alias = "hct")]
    pub horizontal_count: u16,
    #[bits(16..=25, alias = "vct")]
    pub vertical_count: u16,
    #[bits(28, alias = "enb")]
    pub enable: bool,
    #[bits(31, alias = "int")]
    pub interrupt: bool,
}
crate::mmio_reg!(DisplayInterrupt0: u32 @ 0xCC002030);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DisplayInterrupt0 {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.vi.di0
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.vi.di0 = self;
        vi::refresh_interrupts(sys);
    }
}

// 0xCC002034	4	R/W	DI1 (Display Interrupt 1)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DisplayInterrupt1 {
    #[bits(0..=9, alias = "hct")]
    pub horizontal_count: u16,
    #[bits(16..=25, alias = "vct")]
    pub vertical_count: u16,
    #[bits(28, alias = "enb")]
    pub enable: bool,
    #[bits(31, alias = "int")]
    pub interrupt: bool,
}
crate::mmio_reg!(DisplayInterrupt1: u32 @ 0xCC002034);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DisplayInterrupt1 {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.vi.di1
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.vi.di1 = self;
        vi::refresh_interrupts(sys);
    }
}

// 0xCC002038	4	R/W	DI2 (Display Interrupt 2)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DisplayInterrupt2 {
    #[bits(0..=9, alias = "hct")]
    pub horizontal_count: u16,
    #[bits(16..=25, alias = "vct")]
    pub vertical_count: u16,
    #[bits(28, alias = "enb")]
    pub enable: bool,
    #[bits(31, alias = "int")]
    pub interrupt: bool,
}
crate::mmio_reg!(DisplayInterrupt2: u32 @ 0xCC002038);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DisplayInterrupt2 {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.vi.di2
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.vi.di2 = self;
        vi::refresh_interrupts(sys);
    }
}

// 0xCC00203C	4	R/W	DI3 (Display Interrupt 3)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DisplayInterrupt3 {
    #[bits(0..=9, alias = "hct")]
    pub horizontal_count: u16,
    #[bits(16..=25, alias = "vct")]
    pub vertical_count: u16,
    #[bits(28, alias = "enb")]
    pub enable: bool,
    #[bits(31, alias = "int")]
    pub interrupt: bool,
}
crate::mmio_reg!(DisplayInterrupt3: u32 @ 0xCC00203C);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for DisplayInterrupt3 {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.vi.di3
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.vi.di3 = self;
        vi::refresh_interrupts(sys);
    }
}

// 0xCC002040	4	R/W	DL0 (Display Latch 0)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DisplayLatch0 {
    #[bits(0..=10, alias = "hct")]
    pub horizontal_count: u16,
    #[bits(16..=26, alias = "vct")]
    pub vertical_count: u16,
    #[bits(31, alias = "trg")]
    pub trigger: bool,
}
crate::mmio_reg!(DisplayLatch0: u32 @ 0xCC002040);
crate::mmio_default_access!(DisplayLatch0 => System.vi.dl0);

// 0xCC002044	4	R/W	DL1 (Display Latch 1)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct DisplayLatch1 {
    #[bits(0..=10, alias = "hct")]
    pub horizontal_count: u16,
    #[bits(16..=26, alias = "vct")]
    pub vertical_count: u16,
    #[bits(31, alias = "trg")]
    pub trigger: bool,
}
crate::mmio_reg!(DisplayLatch1: u32 @ 0xCC002044);
crate::mmio_default_access!(DisplayLatch1 => System.vi.dl1);

// 0xCC002048	2	R/W	HSW (Horizontal Scaling Width)

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct HorizontalScalingWidth {
    #[bits(0..=9, alias = "srcwidth")]
    pub source_width: u16,
}
crate::mmio_reg!(HorizontalScalingWidth: u16 @ 0xCC002048);
crate::mmio_default_access!(HorizontalScalingWidth => System.vi.hsw);

// 0xCC00204A	2	R/W	HSR (Horizontal Scaling Register)

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct HorizontalScalingRegister {
    #[bits(0..=8, alias = "stp")]
    pub step_size: u16,
    #[bits(12, alias = "hs_en")]
    pub horizontal_scaling_enable: bool,
}
crate::mmio_reg!(HorizontalScalingRegister: u16 @ 0xCC00204A);
crate::mmio_default_access!(HorizontalScalingRegister => System.vi.hsr);

// 0xCC00204C	4	R/W	FCT0 (Filter Coefficient Table 0)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FilterCoefficient0 {
    #[bits(0..=9, alias = "t0")]
    pub tap0: u16,
    #[bits(10..=19, alias = "t1")]
    pub tap1: u16,
    #[bits(20..=29, alias = "t2")]
    pub tap2: u16,
}
crate::mmio_reg!(FilterCoefficient0: u32 @ 0xCC00204C);
crate::mmio_default_access!(FilterCoefficient0 => System.vi.fct0);

// 0xCC002050	4	R/W	FCT1 (Filter Coefficient Table 1)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FilterCoefficient1 {
    #[bits(0..=9, alias = "t3")]
    pub tap3: u16,
    #[bits(10..=19, alias = "t4")]
    pub tap4: u16,
    #[bits(20..=29, alias = "t5")]
    pub tap5: u16,
}
crate::mmio_reg!(FilterCoefficient1: u32 @ 0xCC002050);
crate::mmio_default_access!(FilterCoefficient1 => System.vi.fct1);

// 0xCC002054	4	R/W	FCT2 (Filter Coefficient Table 2)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FilterCoefficient2 {
    #[bits(0..=9, alias = "t6")]
    pub tap6: u16,
    #[bits(10..=19, alias = "t7")]
    pub tap7: u16,
    #[bits(20..=29, alias = "t8")]
    pub tap8: u16,
}
crate::mmio_reg!(FilterCoefficient2: u32 @ 0xCC002054);
crate::mmio_default_access!(FilterCoefficient2 => System.vi.fct2);

// 0xCC002058	4	R/W	FCT3 (Filter Coefficient Table 3)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FilterCoefficient3 {
    #[bits(0..=7, alias = "t9")]
    pub tap9: u8,
    #[bits(8..=15, alias = "t10")]
    pub tap10: u8,
    #[bits(16..=23, alias = "t11")]
    pub tap11: u8,
    #[bits(24..=31, alias = "t12")]
    pub tap12: u8,
}
crate::mmio_reg!(FilterCoefficient3: u32 @ 0xCC002058);
crate::mmio_default_access!(FilterCoefficient3 => System.vi.fct3);

// 0xCC00205C	4	R/W	FCT4 (Filter Coefficient Table 4)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FilterCoefficient4 {
    #[bits(0..=7, alias = "t13")]
    pub tap13: u8,
    #[bits(8..=15, alias = "t14")]
    pub tap14: u8,
    #[bits(16..=23, alias = "t15")]
    pub tap15: u8,
    #[bits(24..=31, alias = "t16")]
    pub tap16: u8,
}
crate::mmio_reg!(FilterCoefficient4: u32 @ 0xCC00205C);
crate::mmio_default_access!(FilterCoefficient4 => System.vi.fct4);

// 0xCC002060	4	R/W	FCT5 (Filter Coefficient Table 5)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FilterCoefficient5 {
    #[bits(0..=7, alias = "t17")]
    pub tap17: u8,
    #[bits(8..=15, alias = "t18")]
    pub tap18: u8,
    #[bits(16..=23, alias = "t19")]
    pub tap19: u8,
    #[bits(24..=31, alias = "t20")]
    pub tap20: u8,
}
crate::mmio_reg!(FilterCoefficient5: u32 @ 0xCC002060);
crate::mmio_default_access!(FilterCoefficient5 => System.vi.fct5);

// 0xCC002064	4	R/W	FCT6 (Filter Coefficient Table 6)

#[chapa::bitfield(u32, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct FilterCoefficient6 {
    #[bits(0..=7, alias = "t21")]
    pub tap21: u8,
    #[bits(8..=15, alias = "t22")]
    pub tap22: u8,
    #[bits(16..=23, alias = "t23")]
    pub tap23: u8,
    #[bits(24..=31, alias = "t24")]
    pub tap24: u8,
}
crate::mmio_reg!(FilterCoefficient6: u32 @ 0xCC002064);
crate::mmio_default_access!(FilterCoefficient6 => System.vi.fct6);

// 0xCC00206C	2	R/W	VICLK (VI Clock Select)

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct ViClockSelect {
    #[bits(0, alias = "clk")]
    pub clock_select: bool,
}
crate::mmio_reg!(ViClockSelect: u16 @ 0xCC00206C);

impl<const SYSTEM: SystemId> MmioAccess<System<SYSTEM>> for ViClockSelect {
    fn read(sys: &mut System<SYSTEM>) -> Self {
        sys.vi.viclk
    }
    fn write(self, sys: &mut System<SYSTEM>, _: WriteMask) {
        sys.vi.viclk = self;
        vi::ensure_half_line_scheduled(sys);
    }
}

// 0xCC00206E	2	R/W	VISEL (VI DTV Status)

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct ViDtvStatus {
    #[bits(2, alias = "visel")]
    pub dtv_status: bool,
}
crate::mmio_reg!(ViDtvStatus: u16 @ 0xCC00206E);
crate::mmio_default_access!(ViDtvStatus => System.vi.visel);

// 0xCC002070	2	R/W	Unknown. Log for now.

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct ViUnknown70 {}
crate::mmio_reg!(ViUnknown70: u16 @ 0xCC002070);
crate::mmio_default_access!(ViUnknown70 => System.vi.unknown_70);

// 0xCC002072	2	R/W	BorderHBE (Border HBE)

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct BorderHbe {
    #[bits(0..=9, alias = "hbe656")]
    pub horizontal_blank_end_656: u16,
    #[bits(15, alias = "brdr_en")]
    pub border_enable: bool,
}
crate::mmio_reg!(BorderHbe: u16 @ 0xCC002072);
crate::mmio_default_access!(BorderHbe => System.vi.border_hbe);

// 0xCC002074	2	R/W	BorderHBS (Border HBS)

#[chapa::bitfield(u16, order = lsb0)]
#[derive(Copy, Clone, Debug)]
pub struct BorderHbs {
    #[bits(0..=9, alias = "hbs656")]
    pub horizontal_blank_start_656: u16,
}
crate::mmio_reg!(BorderHbs: u16 @ 0xCC002074);
crate::mmio_default_access!(BorderHbs => System.vi.border_hbs);
