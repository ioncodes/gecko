pub static IPL_HLE: &[u8] = include_bytes!("../../../submodules/solstice/ipl/hle/ipl/gc_custom_ipl.bin");

#[derive(Clone, Copy)]
pub struct SkipPatch {
    pub name: &'static str,
    pub ipl_hash: u32,
    pub offset: u32,
    pub patch: [u8; 12],
}

// We patch this sequence here. This is the loop that ticks the DVD state machine.
// Once we reach state 16 (BS2_RUN_APP) we jump directly to boot_disc, this
// effectively skips the entire animation/menu flow and boots the disc instead.
//
// do
// {
//   update_ipl_state();
//   wait_retrace();
// }
// while ( get_ipl_state() != 19 && (get_ipl_state() < 5 || get_ipl_state() > 16) && get_ipl_state() != 18 );

pub const SKIP_PATCHES: &[SkipPatch] = &[
    SkipPatch {
        name: "IPL NTSC",
        ipl_hash: 0xB7E67BE1,
        offset: 0x1A7C, // vaddr 0x8130125C
        patch: [
            0x2C, 0x03, 0x00, 0x10, // cmpwi r3, 0x10
            0x40, 0x82, 0xFF, 0xF0, // bne poll BS2 state
            0x4B, 0xFF, 0xF6, 0xD4, // b boot_disc
        ],
    },
    SkipPatch {
        name: "IPL PAL",
        ipl_hash: 0xE1B06B49,
        offset: 0x2B30, // vaddr 0x81302310
        patch: [
            0x2C, 0x00, 0x00, 0x10, // cmpwi r0, 0x10
            0x40, 0x82, 0xFF, 0xEC, // bne poll BS2 state
            0x4B, 0xFF, 0xE5, 0x18, // b boot_disc
        ],
    },
];

pub fn apply_skip_patch(ipl: &mut [u8]) {
    let hash = crc32fast::hash(ipl);
    if let Some(patch) = SKIP_PATCHES.iter().find(|p| p.ipl_hash == hash) {
        tracing::info!("Applying skip patch: {}", patch.name);
        ipl[patch.offset as usize..patch.offset as usize + patch.patch.len()].copy_from_slice(&patch.patch);
    }
}
