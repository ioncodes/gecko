use std::sync::{Arc, Mutex};

use backend_wgpu::GxRenderer;
use gecko::HostInput;
use gecko::gamecube::GameCube;
use gecko::wii::Wii;

use crate::sink::{Introspection, McpSink};
use crate::state::Backend;

pub struct BootResult {
    pub backend: Backend,
    pub game_name: String,
    pub game_code: String,
}

pub fn boot(
    disc_bytes: Vec<u8>,
    ipl: &[u8],
    dsp_rom: &[u8],
    coef_rom: &[u8],
    gx: Arc<Mutex<GxRenderer>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    introspect: Arc<Mutex<Introspection>>,
) -> BootResult {
    let dvd = image::load_dvd(disc_bytes);

    let game_name = String::from_utf8_lossy(&dvd.header().game_name)
        .trim_end_matches('\0')
        .to_owned();
    let game_code = String::from_utf8_lossy(&dvd.header().game_code)
        .trim_end_matches('\0')
        .to_owned();

    let sink = Box::new(McpSink {
        gx,
        device,
        queue,
        introspect,
    });

    let backend = if dvd.header().is_wii() {
        let mut emu = Wii::apploader_hle(dvd).build();
        emu.dsp.load_irom(dsp_rom);
        emu.dsp.load_coef(coef_rom);
        emu.render_sink = sink;
        emu.apply_host_input(&HostInput::wii_neutral());
        Backend::Wii(emu)
    } else {
        let mut emu = GameCube::with_ipl(ipl, true);
        emu.dsp.load_irom(dsp_rom);
        emu.dsp.load_coef(coef_rom);
        emu.render_sink = sink;
        emu.apply_host_input(&HostInput::gc_connected());
        emu.insert_dvd(dvd);
        Backend::Gc(emu)
    };

    BootResult {
        backend,
        game_name,
        game_code,
    }
}
