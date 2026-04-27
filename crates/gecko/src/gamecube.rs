use crate::cpu::IPL_RESET_VECTOR;
use crate::flipper::exi::macronix::ExiMacronix;
#[cfg(feature = "idle-skip")]
use crate::idle::{IDLE_LOOP_MAX_INSTRS, IdleCheck};
use crate::ipl::IPL_HLE;
use crate::scheduler::Scheduler;
use crate::system::{GC, System};
use image::Executable;

pub type GameCube = System<{ GC }>;

impl GameCube {
    pub fn new(entrypoint: u32) -> Self {
        Self::with_scheduler(entrypoint, Scheduler::new_gamecube())
    }

    pub fn with_image(exe: &impl Executable) -> Self {
        let mut emulator = GameCube::new(exe.entry_point());
        emulator.load_image(exe);
        emulator
    }

    pub fn with_ipl_hle(dvd: Box<dyn image::Dvd>) -> Self {
        const APPLOADER_LOAD: u32 = 0x0120_0000;
        const IPL_LOAD: u32 = 0x0130_0000;
        const IPL_ENTRY: u32 = 0x8130_0000;
        const ARAM_SIZE: u32 = 16 * 1024 * 1024;

        let game_name = String::from_utf8_lossy(&dvd.header().game_name);
        let game_name = game_name.trim_end_matches('\0');
        tracing::info!("Game: {game_name}");

        let apploader_version = String::from_utf8_lossy(&dvd.apploader().timestamp);
        tracing::info!("Apploader: {apploader_version}");

        let mut emulator = Self::new(IPL_ENTRY);

        // BATs
        emulator.cpu.spr.dbat0u = 0x8000_1FFF;
        emulator.cpu.spr.dbat0l = 0x0000_0002;
        emulator.cpu.spr.dbat1u = 0xC000_1FFF;
        emulator.cpu.spr.dbat1l = 0x0000_002A;
        emulator.cpu.spr.dbat2u = 0x0000_1FFF;
        emulator.cpu.spr.dbat2l = 0x0000_0002;
        emulator.cpu.spr.dbat3u = 0xFFF0_001F;
        emulator.cpu.spr.dbat3l = 0xFFF0_0001;
        emulator.cpu.spr.ibat0u = 0x8000_1FFF;
        emulator.cpu.spr.ibat0l = 0x0000_0002;
        emulator.cpu.spr.ibat2u = 0x0000_1FFF;
        emulator.cpu.spr.ibat2l = 0x0000_0002;
        emulator.cpu.spr.ibat3u = 0xFFF0_001F;
        emulator.cpu.spr.ibat3l = 0xFFF0_0001;

        // DVD header fields to low memory
        emulator.mmio.ram[0x00..0x04].copy_from_slice(&dvd.header().game_code);
        emulator.mmio.ram[0x04..0x06].copy_from_slice(&dvd.header().maker_code);
        emulator.mmio.ram[0x06] = dvd.header().disk_id;
        emulator.mmio.ram[0x07] = dvd.header().version;
        emulator.mmio.ram[0x08] = dvd.header().audio_streaming;
        emulator.mmio.ram[0x09] = dvd.header().streaming_buffer_size;

        // System info
        emulator
            .mmio
            .phys_write_u32(0x28, crate::mmio::constants::RAM_SIZE as u32);
        emulator.mmio.phys_write_u32(0x2C, 1); // retail console
        emulator.mmio.phys_write_u32(0xD0, ARAM_SIZE);

        // Syscall stub? rfi
        emulator.mmio.phys_write_u32(0x0C00, 0x4C00_0064);

        // MSR: FP + address translation
        emulator.cpu.msr.set_floating_point_available(true);
        emulator.cpu.msr.set_data_address_translation(true);
        emulator.cpu.msr.set_instruction_address_translation(true);

        // ???
        emulator.cpu.spr.hid0 = 0x0011_C464;

        // Load apploader code into RAM
        let apploader_code_start = image::dvd::DVD_APPLOADER_OFFSET + 0x20;
        let apploader_size = (dvd.apploader().size.get() + dvd.apploader().trailer_size.get()) as usize;
        let apploader_entry = dvd.apploader().entrypoint.get();
        dvd.read_disc_into(
            apploader_code_start,
            &mut emulator.mmio.ram[APPLOADER_LOAD as usize..][..apploader_size],
        );

        // Load custom IPL binary into RAM
        emulator.mmio.ram[IPL_LOAD as usize..][..IPL_HLE.len()].copy_from_slice(IPL_HLE);

        // R0 = apploader entry (read by IPL)
        emulator.cpu.gprs[0] = apploader_entry;

        // Insert the DVD
        emulator.insert_dvd(dvd);

        emulator
    }

    pub fn with_ipl(ipl: &[u8], skip: bool) -> Self {
        // Text Sections (1):
        // | idx | offset     | vaddr      | size       | end        |
        // |-----|------------|------------|------------|------------|
        // | 0   | 0x00000100 | 0x81300000 | 0x001FF7E0 | 0x814FF7E0 |
        // Data Sections (0):
        // | idx | offset | vaddr | size | end |
        // |-----|--------|-------|------|-----|
        // Entry point: 0x81300000
        // BSS: 0x00000000 - 0x00000000 (size: 0x00000000)
        // => BS2 DOL, does not apply to the actual IPL here!!

        let mut ipl = ipl.to_vec();
        if skip {
            crate::ipl::apply_skip_patch(&mut ipl);
        }

        let mut emulator = GameCube::new(IPL_RESET_VECTOR);
        emulator.cpu.msr.set_ip(true);
        emulator.mmio.ipl = ipl.clone();
        emulator.exi.attach_device(
            ExiMacronix::CHANNEL,
            ExiMacronix::DEVICE,
            Box::new(ExiMacronix::new(ipl)),
        );
        // TODO: this makes 0x8130107C (NTSC BS2) exit the DVD state machine
        // as it forces it to enter "state 19"
        emulator.open_cover();
        emulator
    }
}
