mod ios;

use crate::scheduler::Scheduler;
use crate::system::{System, WII};
use image::Executable;
use image::dvd::DVD_APPLOADER_OFFSET;

pub type Wii = System<{ WII }>;

const APPLOADER_BASE: u32 = 0x8120_0000;

/// Stack pointer for apploader execution matching Dolphin.
const APPLOADER_STACK: u32 = 0x816F_FFF0;

/// Must sit above every plausible DOL section and above the apploader stack
/// or else you'll be in a world of hurt.
const APPLOADER_ARG_BASE: u32 = 0x8170_0000;

/// Address of the OSReport function pointer passed to apploader's init().
/// Write a `blr` here to stub it out.
const OSREPORT_STUB: u32 = 0x8130_0000;

/// PPC `blr` instruction.
const PPC_BLR: u32 = 0x4E80_0020;

/// PPC `rfi` instruction. Installed at the syscall exception vector so the
/// apploader's `sc` instructions return to caller without trapping.
/// TODO: We have to fix this eventually.
const PPC_RFI: u32 = 0x4C00_0064;

/// Sentinel address loaded into LR before each apploader function call.
/// `run_until` stops as soon as PC reaches this address (set by the function's
/// final `blr`); nothing is ever fetched from here.
const APPLOADER_RETURN_TRAP: u32 = 0x6900_0000;

impl Wii {
    pub fn new(entrypoint: u32) -> Self {
        let mut emu = Self::with_scheduler(entrypoint, Scheduler::new_wii());
        emu.initialize_starlet_devices();
        emu
    }

    pub fn with_image(exe: &impl Executable) -> Self {
        let mut emu = Self::new(exe.entry_point());
        Self::setup_bats(&mut emu);
        Self::setup_msr_hid(&mut emu);
        // No disc / TMD available: default to IOS56, TODO
        Self::setup_low_memory_common(&mut emu, ios::ios56());
        Self::setup_post_ios_handoff(&mut emu);
        emu.load_image(exe);
        emu
    }

    pub fn apploader_hle(dvd: Box<dyn image::Dvd>) -> ApploaderHleBuilder {
        ApploaderHleBuilder {
            dvd,
            #[cfg(feature = "hooks")]
            host: None,
        }
    }

    fn apploader_hle_run(
        dvd: Box<dyn image::Dvd>,
        #[cfg(feature = "hooks")] host: Option<Box<dyn crate::hooks::Host<{ WII }> + Send>>,
    ) -> Self {
        assert!(dvd.header().is_wii(), "apploader HLE only supports Wii discs");

        let game_name = String::from_utf8_lossy(&dvd.header().game_name);
        let game_name = game_name.trim_end_matches('\0');
        tracing::info!("Wii boot: {game_name}");

        let apploader_version = String::from_utf8_lossy(&dvd.apploader().timestamp);
        tracing::info!("Apploader: {apploader_version}");

        let mut emu = Self::new(0);

        Self::setup_bats(&mut emu);
        Self::setup_msr_hid(&mut emu);
        Self::setup_low_memory(&mut emu, dvd.as_ref());
        Self::setup_post_ios_handoff(&mut emu);

        let apploader = *dvd.apploader();
        let apploader_entry = apploader.entrypoint.get();
        let apploader_size = (apploader.size.get() + apploader.trailer_size.get()) as usize;
        let apploader_code_start = DVD_APPLOADER_OFFSET + 0x20;
        tracing::info!(
            addr = format!("{apploader_entry:08X}"),
            size = apploader_size,
            "loading apploader from disc"
        );

        {
            let buffer = emu.mmio.virt_slice_mut(APPLOADER_BASE, apploader_size);
            dvd.read_disc_into(apploader_code_start, buffer);
        }

        emu.mmio.virt_write_u32(OSREPORT_STUB, PPC_BLR);

        // Catch exceptions and return to caller.
        emu.mmio.virt_write_u32(0x8000_0300, PPC_RFI);
        emu.mmio.virt_write_u32(0x8000_0800, PPC_RFI);
        emu.mmio.virt_write_u32(0x8000_0C00, PPC_RFI);

        emu.gekko.gprs[1] = APPLOADER_STACK;

        #[cfg(feature = "hooks")]
        if let Some(host) = host {
            emu.set_hook_host(host);
        }

        Self::run_apploader(&mut emu, dvd.as_ref(), apploader_entry);

        emu.insert_dvd(dvd);
        emu
    }

    fn setup_bats(emu: &mut Wii) {
        emu.gekko.spr.ibat0u = 0x8000_1FFF;
        emu.gekko.spr.ibat0l = 0x0000_0002;
        emu.gekko.spr.dbat0u = 0x8000_1FFF;
        emu.gekko.spr.dbat0l = 0x0000_0002;

        emu.gekko.spr.dbat1u = 0xC000_1FFF;
        emu.gekko.spr.dbat1l = 0x0000_002A;

        emu.gekko.spr.ibat4u = 0x9000_1FFF;
        emu.gekko.spr.ibat4l = 0x1000_0002;
        emu.gekko.spr.dbat4u = 0x9000_1FFF;
        emu.gekko.spr.dbat4l = 0x1000_0002;

        emu.gekko.spr.dbat5u = 0xD000_1FFF;
        emu.gekko.spr.dbat5l = 0x1000_002A;
    }

    fn setup_msr_hid(emu: &mut Wii) {
        emu.gekko.msr.set_floating_point_available(true);
        emu.gekko.msr.set_data_address_translation(true);
        emu.gekko.msr.set_instruction_address_translation(true);

        emu.gekko.spr.hid0 = 0x0011_C664;
        emu.gekko.spr.hid2 = 0xE000_0000;
        emu.gekko.spr.hid4 = 0x8390_0000;
    }

    fn setup_post_ios_handoff(emu: &mut Wii) {
        // Seems to be required for libogc? I guess Starlet leaves IPC mask on.
        emu.hollywood.irq.mask = crate::hollywood::regs::Mask::from_raw(0).with_ipc(true);
    }

    // Cross referenced with beanwii and Dolphin!!
    fn setup_low_memory_common(emu: &mut Wii, imv: &ios::MemoryValues) {
        // Ripped from the apploader.
        emu.mmio
            .phys_write_u32(0x0000_0018, u32::from_be_bytes(image::WII_MAGIC));

        // MEM1 size, board model.
        emu.mmio.phys_write_u32(0x0000_0028, imv.mem1_physical_size);
        emu.mmio.phys_write_u32(0x0000_002C, 0x0000_0023);

        // Exception handlers and IRQ masks.
        emu.mmio.phys_write_u32(0x0000_0048, 0x8134_0000);
        emu.mmio.phys_write_u32(0x0000_00C4, 0xFFFF_FF00);

        // lol.
        emu.mmio.phys_write_u32(0x0000_00E4, 0x8008_F7B8);

        // Bus/CPU speeds.
        emu.mmio.phys_write_u32(0x0000_00F8, 0x0E7B_E2C0);
        emu.mmio.phys_write_u32(0x0000_00FC, 0x2B73_A840);

        // Dolphin shit.
        emu.mmio.phys_write_u32(0x0000_30D8, 0xFFFF_FFFF);

        // ???
        emu.mmio.phys_write_u16(0x0000_30E6, 0x8201);

        // MEM1 layout (kernel writes).
        emu.mmio.phys_write_u32(0x0000_3100, imv.mem1_physical_size);
        emu.mmio.phys_write_u32(0x0000_3104, imv.mem1_simulated_size);
        emu.mmio.phys_write_u32(0x0000_3108, imv.mem1_end);
        emu.mmio.phys_write_u32(0x0000_310C, imv.mem1_arena_begin);
        // 0x3110 is conditionally overwritten by the apploader when the DOL
        // entry is >= 0x80004000; this is the fallback.
        emu.mmio.phys_write_u32(0x0000_3110, imv.mem1_arena_end);

        // MEM2 layout.
        emu.mmio.phys_write_u32(0x0000_3118, imv.mem2_physical_size);
        emu.mmio.phys_write_u32(0x0000_311C, imv.mem2_simulated_size);
        emu.mmio.phys_write_u32(0x0000_3120, imv.mem2_end);
        emu.mmio.phys_write_u32(0x0000_3124, imv.mem2_arena_begin);
        emu.mmio.phys_write_u32(0x0000_3128, imv.mem2_arena_end);

        // IPC buffer (ARM <-> PPC shared region).
        emu.mmio.phys_write_u32(0x0000_3130, imv.ipc_buffer_begin);
        emu.mmio.phys_write_u32(0x0000_3134, imv.ipc_buffer_end);

        // Hollywood revision.
        emu.mmio.phys_write_u32(0x0000_3138, imv.hollywood_revision);

        // IOS version + date + reserved heap.
        emu.mmio.phys_write_u32(0x0000_3140, imv.ios_version);
        emu.mmio.phys_write_u32(0x0000_3144, imv.ios_date);
        emu.mmio.phys_write_u32(0x0000_3148, imv.ios_reserved_begin);
        emu.mmio.phys_write_u32(0x0000_314C, imv.ios_reserved_end);

        // GDDR vendor code.
        emu.mmio.phys_write_u32(0x0000_3158, imv.ram_vendor);

        // Boot flag, devkit boot version.
        emu.mmio.phys_write_u8(0x0000_315C, 0x80);
        emu.mmio.phys_write_u16(0x0000_315E, 0x0113);

        // System menu sync.
        emu.mmio.phys_write_u32(0x0000_3160, imv.sysmenu_sync);

        // App type.
        emu.mmio.phys_write_u8(0x0000_3184, 0x80);

        // Min IOS version requirement.
        emu.mmio.phys_write_u32(0x0000_3188, 0x0035_1011);
    }

    fn setup_low_memory(emu: &mut Wii, dvd: &dyn image::Dvd) {
        let header = dvd.header();
        let game_id = u32::from_be_bytes(header.game_code);
        let maker_code = u16::from_be_bytes(header.maker_code);

        // Disc header copies (from boot1).
        emu.mmio.phys_write_u32(0x0000_0000, game_id);
        emu.mmio.phys_write_u16(0x0000_0004, maker_code);
        emu.mmio.phys_write_u8(0x0000_0006, header.disk_id);
        emu.mmio.phys_write_u8(0x0000_0007, header.version);
        emu.mmio.phys_write_u8(0x0000_0008, header.audio_streaming);
        emu.mmio.phys_write_u8(0x0000_0009, header.streaming_buffer_size);

        let ios_title_id = dvd.tmd_ios_title_id();
        let ios_major = ios_title_id as u16;
        let imv = ios::for_ios(ios_major).unwrap_or_else(|| {
            tracing::warn!(
                ios = ios_major,
                "no MemoryValues row for required IOS, falling back to IOS56"
            );
            ios::ios56()
        });
        tracing::info!(
            ios = ios_major,
            ios_version = format!("{:08X}", imv.ios_version),
            mem2_end = format!("{:08X}", imv.mem2_end),
            "Wii IOS lookup"
        );

        Self::setup_low_memory_common(emu, imv);

        // OSVideoMode: 0 = NTSC, 1 = PAL/MPAL.
        let video_mode: u32 = if header.is_ntsc() { 0 } else { 1 };
        emu.mmio.phys_write_u32(0x0000_00CC, video_mode);

        // Game ID dup.
        emu.mmio.phys_write_u32(0x0000_3180, game_id);

        // Data partition magic + offset (Sonic Colors / NSMBW read these).
        emu.mmio.phys_write_u32(0x0000_3194, 0x8000_0000);
        emu.mmio
            .phys_write_u32(0x0000_3198, (dvd.data_partition_offset() >> 2) as u32);
    }

    fn run_apploader(emu: &mut Wii, dvd: &dyn image::Dvd, apploader_entry: u32) {
        let call_function = |emu: &mut Wii, entry: u32| {
            emu.gekko.spr.lr = APPLOADER_RETURN_TRAP;
            emu.run_until(entry, |sys| sys.gekko.pc == APPLOADER_RETURN_TRAP);
        };

        // void __fastcall Apploader_Entry(void **init_func, void **main_func, void **final_func)
        // {
        //   *init_func = (void *)0x81200470;
        //   *main_func = (void *)0x81200490;
        //   *final_func = (void *)0x812004B0;
        //   memmove((void *)0x8132FF80, (const void *)0x81200420, 0x4Cu);
        //   DCStoreRange((void *)0x8132FF80, 0x4Cu);
        //   ICInvalidateRange((void *)0x8132FF80, 0x4Cu);
        // }
        emu.gekko.gprs[3] = APPLOADER_ARG_BASE;
        emu.gekko.gprs[4] = APPLOADER_ARG_BASE + 4;
        emu.gekko.gprs[5] = APPLOADER_ARG_BASE + 8;
        call_function(emu, apploader_entry);

        let init_ptr = emu.read_u32(APPLOADER_ARG_BASE);
        let main_ptr = emu.read_u32(APPLOADER_ARG_BASE + 4);
        let close_ptr = emu.read_u32(APPLOADER_ARG_BASE + 8);
        tracing::info!(
            init = format!("{init_ptr:08X}"),
            main = format!("{main_ptr:08X}"),
            close = format!("{close_ptr:08X}"),
            "Apploader_Entry() returned"
        );
        assert!(init_ptr != 0 && main_ptr != 0 && close_ptr != 0);

        // unsigned int __fastcall Apploader_Init(void (*report)(const char *fmt, ...))
        // {
        //   unsigned int result; // r3
        //   memset((void *)0x81201C60, 0, 0x20u);
        //   memset((void *)0x81201C80, 0, 0x100u);
        //   MEMORY[0x81201D80] = 0;
        //   MEMORY[0x81201D84] = 0;
        //   MEMORY[0x81201D88] = 0;
        //   MEMORY[0x81201C58] = (int (__fastcall *)(_DWORD))report;
        //   MEMORY[0x81201C54] = 2;
        //   copy_lowmem_boot_info((void *)0x81201D8C);
        //   MEMORY[0x81201C58](-2128603796);
        //   MEMORY[0x81201C58](-2128603768);
        //   MEMORY[0x8000315D] = 0x80;
        //   if ( MEMORY[0x8000315E] < 0x107u )
        //     MEMORY[0x81201C58](-2128603704);
        //   MEMORY[0x80003188] = 3674906;
        //   result = MEMORY[0x80000018];
        //   if ( MEMORY[0x80000018] == 1562156707 )
        //     MEMORY[0x81201C54] = 0;
        //   else
        //     MEMORY[0x81201C54] = 2;
        //   return result;
        // }
        emu.gekko.gprs[3] = OSREPORT_STUB;
        call_function(emu, init_ptr);
        tracing::info!("Apploader_Init() returned");

        // Apploader_Main() loads text, data, set up OS globals, etc.
        // Returns 1 in r3 if there is more work to do
        loop {
            emu.gekko.gprs[3] = APPLOADER_ARG_BASE;
            emu.gekko.gprs[4] = APPLOADER_ARG_BASE + 4;
            emu.gekko.gprs[5] = APPLOADER_ARG_BASE + 8;
            call_function(emu, main_ptr);

            if emu.gekko.gprs[3] == 0 {
                break;
            }

            let addr = emu.read_u32(APPLOADER_ARG_BASE);
            let length = emu.read_u32(APPLOADER_ARG_BASE + 4) as usize;
            let offset = (emu.read_u32(APPLOADER_ARG_BASE + 8) as u64) << 2;
            tracing::info!(
                dst = format!("{addr:08X}"),
                len = length,
                offset = format!("{offset:X}"),
                "Apploader_Main() disc read"
            );

            let buffer = emu.mmio.virt_slice_mut(addr, length);
            dvd.read_disc_into(offset as usize, buffer);
        }

        // u32 Apploader_ReturnEpilogue()
        // {
        //   return g_apploader_dol.entry_point;
        // }
        call_function(emu, close_ptr);
        let entrypoint = emu.gekko.gprs[3];
        assert!(entrypoint != 0, "Apploader_ReturnEpilogue() returned null entrypoint");
        emu.gekko.pc = entrypoint;
        tracing::info!(
            entrypoint = format!("{entrypoint:08X}"),
            "Apploader_ReturnEpilogue() returned, ready to run game"
        );
    }
}

pub struct ApploaderHleBuilder {
    dvd: Box<dyn image::Dvd>,
    #[cfg(feature = "hooks")]
    host: Option<Box<dyn crate::hooks::Host<{ WII }> + Send>>,
}

impl ApploaderHleBuilder {
    #[cfg(feature = "hooks")]
    pub fn lua_host(mut self, host: Box<dyn crate::hooks::Host<{ WII }> + Send>) -> Self {
        self.host = Some(host);
        self
    }

    pub fn build(self) -> Wii {
        Wii::apploader_hle_run(
            self.dvd,
            #[cfg(feature = "hooks")]
            self.host,
        )
    }
}
