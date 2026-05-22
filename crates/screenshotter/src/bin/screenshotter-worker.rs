use backend_wgpu::{GxRenderer, capture};
use gecko::HostInput;
use gecko::flipper::si::pad;
use gecko::flipper::vi::regs::RefreshRate;
use gecko::gamecube::GameCube;
use gecko::hollywood::ipc::usb as wiimote;
use gecko::host::{GxAction, RenderSink};
use gecko::system::{System, SystemId};
use gecko::wii::Wii;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const IPL: &[u8] = include_bytes!("../../../../private/IPL.decoded.bin");
const DSP: &[u8] = include_bytes!("../../../../private/dsp_rom.bin");
const COEF: &[u8] = include_bytes!("../../../../private/dsp_coef.bin");

struct SyncSink {
    gx: Arc<Mutex<GxRenderer>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    scratch: Vec<gecko::host::DrawVertex>,
}

impl RenderSink for SyncSink {
    fn exec(&mut self, action: GxAction) {
        self.gx.lock().unwrap().process_action_with_external_scratch(
            &self.device,
            &self.queue,
            &action,
            &mut self.scratch,
        );
    }

    fn vertex_scratch(&mut self) -> &mut Vec<gecko::host::DrawVertex> {
        &mut self.scratch
    }
}

fn take_screenshot(device: &wgpu::Device, queue: &wgpu::Queue, gx: &GxRenderer, code: &str, frame: u32) {
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });

    let mut captured = capture::capture_texture(device, queue, &gx.xfb_texture).expect("capture_texture returned None");

    for px in captured.rgba.chunks_exact_mut(4) {
        px[3] = 255;
    }

    let path = format!("screenshotdb/{}/{}.png", code, frame);
    let file = std::fs::File::create(&path).expect("Failed to create PNG file");

    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), captured.width, captured.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);

    let mut writer = encoder.write_header().expect("Failed to write PNG header");
    writer
        .write_image_data(&captured.rgba)
        .expect("Failed to write PNG data");
}

fn main() {
    let file = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("worker requires a path to a single ISO/RVZ/ZIP"),
    );

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no compatible wgpu adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .expect("failed to acquire wgpu device");

    let surface_format = wgpu::TextureFormat::Rgba8Unorm;

    run_one(&device, &queue, surface_format, &file);
}

fn run_one(device: &wgpu::Device, queue: &wgpu::Queue, surface_format: wgpu::TextureFormat, file: &std::path::Path) {
    let buffer = std::fs::read(file).expect("Failed to read the file");
    let image = image::load_dvd(buffer);

    let name = String::from_utf8_lossy(&image.header().game_name);
    let name = name.trim_end_matches('\0').to_owned();
    let code = String::from_utf8_lossy(&image.header().game_code);
    let code = code.trim_end_matches('\0').to_owned();
    let is_wii = image.header().is_wii();
    println!("Running: {} ({}) [{}]", name, code, if is_wii { "Wii" } else { "GC" });

    let out_dir = format!("screenshotdb/{}", code);
    std::fs::create_dir_all(&out_dir).expect("Failed to create screenshotdb directory");

    let gx = Arc::new(Mutex::new(GxRenderer::new(device, queue, surface_format)));
    let sink = SyncSink {
        gx: gx.clone(),
        device: device.clone(),
        queue: queue.clone(),
        scratch: Vec::new(),
    };

    if is_wii {
        let mut wii = Wii::apploader_hle(image).build();
        wii.dsp.load_irom(DSP);
        wii.dsp.load_coef(COEF);
        wii.render_sink = Box::new(sink);
        drive(
            &mut wii,
            device,
            queue,
            &gx,
            &code,
            HostInput::wii_neutral(),
            update_wii_input,
        );
    } else {
        let mut gamecube = GameCube::with_ipl(IPL, true);
        gamecube.dsp.load_irom(DSP);
        gamecube.dsp.load_coef(COEF);
        gamecube.render_sink = Box::new(sink);
        gamecube.insert_dvd(image);
        drive(
            &mut gamecube,
            device,
            queue,
            &gx,
            &code,
            HostInput::gc_connected(),
            update_gc_input,
        );
    }
}

fn drive<const SYSTEM: SystemId>(
    emu: &mut System<SYSTEM>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gx: &Arc<Mutex<GxRenderer>>,
    code: &str,
    mut input: HostInput,
    mut update_input: impl FnMut(&mut HostInput, usize),
) {
    emu.apply_host_input(&input);

    let framerate = match emu.vi.dcr.video_format().refresh_rate() {
        RefreshRate::Hz50 => 50,
        RefreshRate::Hz60 => 60,
    };

    // Preliminary for IPL skip / boot settle.
    for _ in 0..framerate {
        emu.run_until_vsync();
    }

    let mut frame: u32 = framerate * 2;
    {
        let g = gx.lock().unwrap();
        take_screenshot(device, queue, &g, code, frame);
    }

    for idx in 0..20 {
        update_input(&mut input, idx);
        emu.apply_host_input(&input);

        for _ in 0..(framerate * 2) {
            emu.run_until_vsync();
            frame += 1;
        }

        let g = gx.lock().unwrap();
        take_screenshot(device, queue, &g, code, frame);
    }
}

fn update_gc_input(input: &mut HostInput, idx: usize) {
    let HostInput::Gc(pad) = input else {
        return;
    };
    pad.stick_y = pad::STICK_CENTER;
    pad.buttons = if idx % 2 == 0 { pad::A } else { pad::START };
}

fn update_wii_input(input: &mut HostInput, idx: usize) {
    let HostInput::Wii {
        wiimote_buttons,
        nunchuk_buttons,
        nunchuk_stick_x,
        nunchuk_stick_y,
        ..
    } = input
    else {
        return;
    };
    *nunchuk_buttons = 0;
    *nunchuk_stick_x = wiimote::NUNCHUK_STICK_CENTER;
    *nunchuk_stick_y = wiimote::NUNCHUK_STICK_CENTER;
    *wiimote_buttons = if idx % 2 == 0 {
        wiimote::BTN_A
    } else {
        wiimote::BTN_PLUS
    };
}
