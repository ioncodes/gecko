// DSP DMA registers
pub const IFX_DSCR: u16 = 0xFFC9;
pub const IFX_DSBL: u16 = 0xFFCB;
pub const IFX_DSPA: u16 = 0xFFCD;
pub const IFX_DSMAH: u16 = 0xFFCE;
pub const IFX_DSMAL: u16 = 0xFFCF;

// Accelerator registers
pub const IFX_FORMAT: u16 = 0xFFD1;
pub const IFX_ACDRAW: u16 = 0xFFD3;
pub const IFX_ACSAH: u16 = 0xFFD4;
pub const IFX_ACSAL: u16 = 0xFFD5;
pub const IFX_ACEAH: u16 = 0xFFD6;
pub const IFX_ACEAL: u16 = 0xFFD7;
pub const IFX_ACCAH: u16 = 0xFFD8;
pub const IFX_ACCAL: u16 = 0xFFD9;
pub const IFX_PRED_SCALE: u16 = 0xFFDA;
pub const IFX_YN1: u16 = 0xFFDB;
pub const IFX_YN2: u16 = 0xFFDC;
pub const IFX_ACDSAMP: u16 = 0xFFDD;
pub const IFX_GAIN: u16 = 0xFFDE;
pub const IFX_ACIN: u16 = 0xFFDF;

// ARAM DMA request mask
pub const IFX_AMDM: u16 = 0xFFEF;

// Mailbox registers
pub const IFX_DIRQ: u16 = 0xFFFB;
pub const IFX_DMBH: u16 = 0xFFFC;
pub const IFX_DMBL: u16 = 0xFFFD;
pub const IFX_CMBH: u16 = 0xFFFE;
pub const IFX_CMBL: u16 = 0xFFFF;
