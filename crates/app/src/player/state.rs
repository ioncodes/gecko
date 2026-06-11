use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use backend_wgpu::capture::CapturedFrame;
use backend_wgpu::sink::{Renderer, TargetAspect};
use gecko::audio::EmptyAudioSink;
use gecko::fps::{self, FpsShared};
use gecko::gamecube::GameCube;
use gecko::hollywood::ipc::usb;
use gecko::wii::Wii;
use gecko::{HostInput, system};
use iced::keyboard::key::{Code, Physical};
use iced::mouse::Button as MouseButton;

use crate::config::{Config, DSP_COEF_FILE, DSP_ROM_FILE, IPL_FILE};
use crate::game::{Format, Game, Platform};
use crate::player::{audio, emu_thread, input};

struct BootParams {
    disc_path: PathBuf,
    format: Format,
    dsp_path: Option<PathBuf>,
    coef_path: Option<PathBuf>,
    ipl_path: Option<PathBuf>,
    skip_ipl: bool,
    execution_mode: gecko::ExecutionMode,
    input_config: hostinput::InputConfig,
}

struct Initialized {
    renderer: Renderer,
    _audio_stream: Mutex<Option<cpal::Stream>>,
}

pub struct PlayerState {
    boot: Mutex<Option<BootParams>>,
    initialized: OnceLock<Initialized>,
    shutdown: Arc<AtomicBool>,
    throttle: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    aspect: TargetAspect,
    upscale: u32,
    input: Arc<Mutex<HostInput>>,
    platform: Platform,
    boot_error: Option<String>,
    first_frame: Arc<AtomicBool>,
    fps: FpsShared,
}

#[derive(Debug, Clone)]
pub enum PlayerStatus {
    Booting,
    Failed(String),
    Ready,
}

impl std::fmt::Debug for PlayerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayerState").field("platform", &self.platform).finish()
    }
}

impl PlayerState {
    pub fn new(game: &Game, config: &Config) -> Arc<Self> {
        let aspect = match game.platform {
            Platform::Wii => TargetAspect::Ratio(16.0 / 9.0),
            Platform::Gcn => TargetAspect::Ratio(4.0 / 3.0),
        };
        let neutral = match game.platform {
            Platform::Wii => HostInput::wii_neutral(),
            Platform::Gcn => HostInput::gc_connected(),
        };

        let system_dir = config.system_dir_resolved();
        let dsp = Config::resolve_in_dir(&config.dsp_rom, &system_dir, DSP_ROM_FILE);
        let coef = Config::resolve_in_dir(&config.dsp_coef, &system_dir, DSP_COEF_FILE);
        let ipl = Config::resolve_in_dir(&config.ipl, &system_dir, IPL_FILE);
        let boot_error = self::validate(game.platform, game.format, &dsp, &coef, &ipl, &system_dir);

        Arc::new(Self {
            boot: Mutex::new(Some(BootParams {
                disc_path: game.path.clone(),
                format: game.format,
                dsp_path: dsp,
                coef_path: coef,
                ipl_path: ipl,
                skip_ipl: config.skip_ipl,
                execution_mode: config.cpu_mode.into(),
                input_config: config.input.clone(),
            })),
            initialized: OnceLock::new(),
            shutdown: Arc::new(AtomicBool::new(false)),
            throttle: Arc::new(AtomicBool::new(true)),
            paused: Arc::new(AtomicBool::new(false)),
            aspect,
            upscale: config.upscale,
            input: Arc::new(Mutex::new(neutral)),
            platform: game.platform,
            boot_error,
            first_frame: Arc::new(AtomicBool::new(false)),
            fps: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn status(&self) -> PlayerStatus {
        if let Some(err) = &self.boot_error {
            return PlayerStatus::Failed(err.clone());
        }
        if self.first_frame.load(Ordering::Relaxed) {
            return PlayerStatus::Ready;
        }
        PlayerStatus::Booting
    }

    pub fn blit(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: (u32, u32),
        load: wgpu::LoadOp<wgpu::Color>,
    ) {
        if let Some(init) = self.initialized.get() {
            init.renderer.blit_into_encoder(encoder, target, size, load);
        }
    }

    pub fn start_boot(this: &Arc<Self>, device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) {
        if this.boot_error.is_some() || this.initialized.get().is_some() {
            return;
        }
        let Some(params) = this.boot.lock().unwrap().take() else {
            return;
        };

        let device = device.clone();
        let queue = queue.clone();
        let state = this.clone();
        std::thread::Builder::new()
            .name("gecko-player".into())
            .spawn(move || self::player_thread(state, params, device, queue, format))
            .expect("spawn player thread");
    }

    pub fn handle_keyboard(&self, key: Code, pressed: bool) {
        let mut input_guard = self.input.lock().unwrap();
        match &mut *input_guard {
            HostInput::Gc(pad) => input::update_pad(pad, key, pressed),
            HostInput::Wii {
                wiimote_buttons,
                wiimote_shake,
                nunchuk_buttons,
                nunchuk_stick_x,
                nunchuk_stick_y,
                ..
            } => {
                input::update_wiimote_keys(wiimote_buttons, key, pressed);
                input::update_wiimote_motion_keys(wiimote_shake, key, pressed);
                input::update_nunchuk_keys(nunchuk_buttons, nunchuk_stick_x, nunchuk_stick_y, key, pressed);
            }
        }
    }

    pub fn handle_mouse_button(&self, button: MouseButton, pressed: bool) {
        let mask = match button {
            MouseButton::Left => usb::BTN_A,
            MouseButton::Right => usb::BTN_B,
            _ => return,
        };

        let mut input_guard = self.input.lock().unwrap();
        if let HostInput::Wii { wiimote_buttons, .. } = &mut *input_guard {
            if pressed {
                *wiimote_buttons |= mask;
            } else {
                *wiimote_buttons &= !mask;
            }
        }
    }

    pub fn set_ir_pointer(&self, aim_x: f32, aim_y: f32) {
        if self.platform != Platform::Wii {
            return;
        }
        let (ir_x, ir_y) = gecko::input::aim_to_ir(aim_x, aim_y);
        let mut input_guard = self.input.lock().unwrap();
        if let HostInput::Wii { ir_pointer, .. } = &mut *input_guard {
            *ir_pointer = Some((ir_x, ir_y));
        }
    }

    pub fn clear_ir_pointer(&self) {
        if self.platform != Platform::Wii {
            return;
        }
        let mut input_guard = self.input.lock().unwrap();
        if let HostInput::Wii { ir_pointer, .. } = &mut *input_guard {
            *ir_pointer = None;
        }
    }

    pub fn fps(&self) -> (f32, f32) {
        fps::read(&self.fps)
    }

    pub fn capture_frame(&self) -> Option<CapturedFrame> {
        self.initialized.get().and_then(|init| init.renderer.capture_xfb())
    }

    pub fn toggle_throttle(&self) -> bool {
        !self.throttle.fetch_xor(true, Ordering::Relaxed)
    }

    pub fn toggle_pause(&self) {
        self.paused.fetch_xor(true, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

impl Drop for PlayerState {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

pub fn physical_to_code(physical: &Physical) -> Option<Code> {
    match physical {
        Physical::Code(code) => Some(*code),
        Physical::Unidentified(_) => None,
    }
}

fn player_thread(
    state: Arc<PlayerState>,
    params: BootParams,
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
) {
    tracing::warn!(?format, path = %params.disc_path.display(), "player booting");

    let data = match std::fs::read(&params.disc_path) {
        Ok(b) => b,
        Err(err) => {
            tracing::error!(?err, path = %params.disc_path.display(), "read disc");
            return;
        }
    };
    if state.shutdown.load(Ordering::Relaxed) {
        return;
    }

    let (renderer, sink) = Renderer::new(device, queue, format, state.aspect, state.upscale);
    {
        let first_frame = state.first_frame.clone();
        renderer.set_frame_ready_callback(move |_| {
            first_frame.store(true, Ordering::Relaxed);
        });
    }

    if params.format == Format::Dol {
        let game_id = gecko::jit::cache::dol_cache_id(&data);
        let dol = image::Dol::parse(data);
        match state.platform {
            Platform::Wii => self::finish_boot(state, params, renderer, sink, Wii::with_image(&dol), game_id),
            Platform::Gcn => self::finish_boot(state, params, renderer, sink, GameCube::with_image(&dol), game_id),
        }
        return;
    }

    let dvd = image::load_dvd(data);
    let game_id = dvd.header().game_id();

    match state.platform {
        Platform::Wii => self::finish_boot(state, params, renderer, sink, Wii::apploader_hle(dvd).build(), game_id),
        Platform::Gcn => {
            let emu = self::build_gamecube(dvd, &params);
            self::finish_boot(state, params, renderer, sink, emu, game_id);
        }
    }
}

fn finish_boot<const S: gecko::system::SystemId>(
    state: Arc<PlayerState>,
    params: BootParams,
    renderer: Renderer,
    sink: backend_wgpu::sink::ThreadedSink,
    mut emu: gecko::system::System<S>,
    game_id: String,
) {
    self::configure_emu(&mut emu, &params, sink, state.fps.clone());
    emu.apply_host_input(&HostInput::neutral_for(S));

    let audio = self::install_audio_sink(&mut emu);
    let _ = emu.load_jit_cache(&game_id);
    self::set_initialized(&state, renderer, audio, &game_id);

    emu_thread::run::<S>(
        emu,
        state.input.clone(),
        params.input_config,
        Some(game_id),
        state.throttle.clone(),
        state.paused.clone(),
        state.shutdown.clone(),
    );
}

fn build_gamecube(dvd: Box<dyn image::Dvd>, params: &BootParams) -> gecko::system::System<{ system::GC }> {
    let Some(ipl_path) = params.ipl_path.as_deref() else {
        return GameCube::with_ipl_hle(dvd);
    };
    match std::fs::read(ipl_path) {
        Ok(ipl_data) => {
            tracing::warn!(path = %ipl_path.display(), skip_ipl = params.skip_ipl, "booting via real IPL");
            let mut emu = GameCube::with_ipl(&ipl_data, params.skip_ipl);
            emu.insert_dvd(dvd);
            emu
        }
        Err(err) => {
            tracing::warn!(?err, path = %ipl_path.display(), "IPL.bin unreadable; falling back to IPL HLE");
            GameCube::with_ipl_hle(dvd)
        }
    }
}

fn configure_emu<const S: gecko::system::SystemId>(
    emu: &mut gecko::system::System<S>,
    params: &BootParams,
    sink: backend_wgpu::sink::ThreadedSink,
    fps: FpsShared,
) {
    emu.set_execution_mode(params.execution_mode);
    self::load_dsp_roms(emu, params.dsp_path.as_deref(), params.coef_path.as_deref());
    emu.render_sink = Box::new(sink);
    emu.fps_counter.shared = fps;
}

fn set_initialized(state: &Arc<PlayerState>, renderer: Renderer, audio: Option<cpal::Stream>, game_id: &str) {
    let _ = state.initialized.set(Initialized {
        renderer,
        _audio_stream: Mutex::new(audio),
    });
    tracing::warn!(game = %game_id, "player boot complete");
}

fn validate(
    platform: Platform,
    format: Format,
    dsp: &Option<PathBuf>,
    coef: &Option<PathBuf>,
    ipl: &Option<PathBuf>,
    system_dir: &Path,
) -> Option<String> {
    let mut missing: Vec<&'static str> = Vec::new();
    if dsp.is_none() {
        missing.push(DSP_ROM_FILE);
    }
    if coef.is_none() {
        missing.push(DSP_COEF_FILE);
    }
    if platform == Platform::Gcn && format != Format::Dol && ipl.is_none() {
        missing.push(IPL_FILE);
    }

    if missing.is_empty() {
        return None;
    }

    let needed = if format == Format::Dol {
        "DOL executables require dsp_rom.bin and dsp_coef.bin"
    } else {
        match platform {
            Platform::Gcn => "GameCube games require IPL.bin, dsp_rom.bin and dsp_coef.bin",
            Platform::Wii => "Wii games require dsp_rom.bin and dsp_coef.bin",
        }
    };

    Some(format!(
        "{needed}.\n\nMissing: {}\n\nPlace them under {} or set explicit paths in config.toml.",
        missing.join(", "),
        system_dir.display(),
    ))
}

fn load_dsp_roms<const S: gecko::system::SystemId>(
    emulator: &mut gecko::system::System<S>,
    dsp_path: Option<&Path>,
    coef_path: Option<&Path>,
) {
    if let Some(path) = dsp_path {
        match std::fs::read(path) {
            Ok(data) => emulator.dsp.load_irom(&data),
            Err(err) => tracing::warn!(?err, path = %path.display(), "failed to load DSP IROM"),
        }
    }

    if let Some(path) = coef_path {
        match std::fs::read(path) {
            Ok(data) => emulator.dsp.load_coef(&data),
            Err(err) => tracing::warn!(?err, path = %path.display(), "failed to load DSP coef ROM"),
        }
    }
}

fn install_audio_sink<const S: gecko::system::SystemId>(
    emulator: &mut gecko::system::System<S>,
) -> Option<cpal::Stream> {
    let emulated_rate = emulator.ai.control.aid_sample_rate_hz();
    match audio::open(emulated_rate) {
        Ok(backend) => {
            emulator.audio_sink = Box::new(backend.sink);
            Some(backend.stream)
        }
        Err(err) => {
            tracing::warn!(?err, "failed to open CPAL output; running silent");
            emulator.audio_sink = Box::new(EmptyAudioSink);
            None
        }
    }
}
