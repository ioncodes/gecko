/// Maximum backward branch distance (in bytes) considered a potential polling loop.
pub const IDLE_LOOP_MAX_SIZE: u32 = 10 * 4;
pub const IDLE_LOOP_MAX_INSTRS: usize = (IDLE_LOOP_MAX_SIZE / 4 + 1) as usize;
/// Number of consecutive loop iterations before we attempt to skip.
const IDLE_LOOP_THRESHOLD: u32 = 10;

const MMIO_VIRT_START: u32 = 0xCC00_0000;
const MMIO_VIRT_END: u32 = 0xCC00_FFFF;

pub enum IdleCheck {
    /// Nothing special, keep executing normally.
    Continue,
    /// Branch-to-self detected, always safe to skip to the next event.
    Skip,
    /// A backward-branch loop hit the threshold, caller must validate the loop
    /// body and call [`IdleDetector::set_validated`] with the result.
    Validate { start: u32, end: u32 },
}

pub struct IdleDetector {
    loop_pc: Option<u32>,
    loop_end: u32,
    loop_count: u32,
    validated: LoopStatus,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoopStatus {
    /// Not yet checked.
    Unknown,
    /// Safe polling loop, skip allowed.
    Safe,
    /// Has side-effects, never skip.
    Unsafe,
}

impl IdleDetector {
    pub fn new() -> Self {
        Self {
            loop_pc: None,
            loop_end: 0,
            loop_count: 0,
            validated: LoopStatus::Unknown,
        }
    }

    #[inline(always)]
    pub fn check(&mut self, cia: u32, nia: u32) -> IdleCheck {
        // Branching to current is always a safe idle loop
        if nia == cia {
            return IdleCheck::Skip;
        }

        // Short backward branch, potential polling loop
        if nia < cia && cia.wrapping_sub(nia) <= IDLE_LOOP_MAX_SIZE {
            return self.track_backward_branch(nia, cia);
        }

        self.try_reset(cia);
        IdleCheck::Continue
    }

    /// Record that the current loop was validated (or not) as a safe polling
    /// loop. The result is cached until the loop changes.
    pub fn set_validated(&mut self, safe: bool) {
        self.validated = if safe { LoopStatus::Safe } else { LoopStatus::Unsafe };
    }

    fn track_backward_branch(&mut self, target: u32, branch_pc: u32) -> IdleCheck {
        if self.loop_pc == Some(target) {
            self.loop_count += 1;
            if self.loop_count >= IDLE_LOOP_THRESHOLD {
                self.loop_count = 0;
                return match self.validated {
                    LoopStatus::Safe => IdleCheck::Skip,
                    LoopStatus::Unsafe => IdleCheck::Continue,
                    LoopStatus::Unknown => IdleCheck::Validate {
                        start: target,
                        end: branch_pc,
                    },
                };
            }
        } else {
            // New loop
            self.loop_pc = Some(target);
            self.loop_end = branch_pc;
            self.loop_count = 1;
            self.validated = LoopStatus::Unknown;
        }
        IdleCheck::Continue
    }

    fn try_reset(&mut self, cia: u32) {
        if let Some(start) = self.loop_pc {
            if cia < start || cia > self.loop_end {
                self.loop_pc = None;
                self.loop_count = 0;
                self.validated = LoopStatus::Unknown;
            }
        }
    }
}

/// Analyse a small backward-branch loop and decide whether it is a pure
/// MMIO-polling loop that is safe to skip.
///
/// Requirements:
/// * Every instruction is side effect free (no stores, no calls, no
///   supervisor stuff).
/// * At least one load targets the hardware register address range
///   (`0xCC00_0000 ..= 0xCC00_FFFF`).
pub fn validate_polling_loop(instrs: &[u32], gprs: &[u32; 32]) -> bool {
    let mut has_mmio_load = false;

    for &raw in instrs {
        let opcode = raw >> 26;
        match opcode {
            // D-form integer loads
            // lwz, lwzu, lbz, lbzu, lhz, lhzu, lha, lhau
            32 | 33 | 34 | 35 | 40 | 41 | 42 | 43 => {
                has_mmio_load |= d_form_targets_mmio(raw, gprs);
            }
            // D-form FP loads (lfs, lfsu, lfd, lfdu)
            48 | 49 | 50 | 51 => {}
            // Load multiple word
            46 => {}
            // ALU immediate / compares
            // mulli, subfic, cmpli, cmpi, addic, addic., addi, addis
            7 | 8 | 10 | 11 | 12 | 13 | 14 | 15 => {}
            // ori, oris, xori, xoris, andi., andis.
            24 | 25 | 26 | 27 | 28 | 29 => {}
            // rlwimi, rlwinm, rlwnm
            20 | 21 | 23 => {}
            // FP arith (single / double)
            59 | 63 => {}
            // Branches (without link)
            16 | 18 => {
                if raw & 1 != 0 {
                    return false; // bl / bcl = function call
                }
            }
            // Extended integer ops (opcode 31)
            31 => {
                let xo = (raw >> 1) & 0x3FF;
                if is_unsafe_xo_31(xo) {
                    return false;
                }
                if is_indexed_load_xo(xo) {
                    has_mmio_load |= x_form_targets_mmio(raw, gprs);
                }
            }
            // Rest
            _ => return false,
        }
    }

    has_mmio_load
}

/// Check the effective address of a D-form load (rA + sign-ext d).
fn d_form_targets_mmio(raw: u32, gprs: &[u32; 32]) -> bool {
    let ra = ((raw >> 16) & 0x1F) as usize;
    let d = (raw & 0xFFFF) as i16 as i32;
    let base = if ra == 0 { 0u32 } else { gprs[ra] };
    let ea = (base as i32).wrapping_add(d) as u32;
    (MMIO_VIRT_START..=MMIO_VIRT_END).contains(&ea)
}

/// Check the effective address of an X-form indexed load (rA + rB).
fn x_form_targets_mmio(raw: u32, gprs: &[u32; 32]) -> bool {
    let ra = ((raw >> 16) & 0x1F) as usize;
    let rb = ((raw >> 11) & 0x1F) as usize;
    let base = if ra == 0 { 0u32 } else { gprs[ra] };
    let ea = base.wrapping_add(gprs[rb]);
    (MMIO_VIRT_START..=MMIO_VIRT_END).contains(&ea)
}

/// Opcode-31 extended ops that have side-effects (stores, supervisor writes,
/// I/O writes, cache-invalidate).
fn is_unsafe_xo_31(xo: u32) -> bool {
    matches!(
        xo,
        // Integer stores
        150 | 151 | 183 | 215 | 247 | 407 | 439 | 661 | 662 | 725 | 918
        // FP stores
        | 663 | 695 | 727 | 759 | 983
        // dcbz
        | 1014
        // Supervisor / SPR writes
        | 146  // mtmsr
        | 210  // mtsr
        | 242  // mtsrin
        | 467  // mtspr
        // I/O write
        | 438 // ecowx
    )
}

/// Opcode-31 X-form indexed loads.
fn is_indexed_load_xo(xo: u32) -> bool {
    matches!(
        xo,
        23   // lwzx
        | 55   // lwzux
        | 87   // lbzx
        | 119  // lbzux
        | 279  // lhzx
        | 311  // lhzux
        | 343  // lhax
        | 375  // lhaux
        | 534  // lwbrx
        | 790 // lhbrx
    )
}
