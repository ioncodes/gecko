use crate::device::{Button, Capabilities, DeviceState, PortInput};
use crate::motion::GyroPointer;
use sdl3::GamepadSubsystem;
use sdl3::event::Event;
use sdl3::gamepad::{Axis, Button as SdlButton, Gamepad};
use sdl3::joystick::JoystickId;
use sdl3::sensor::SensorType;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const STANDARD_GRAVITY: f32 = 9.80665;

#[derive(Clone, Copy)]
pub struct MotionSettings {
    pub sensitivity: f32,
    pub recenter: Button,
    pub invert: (bool, bool),
}

impl Default for MotionSettings {
    fn default() -> Self {
        Self {
            sensitivity: 1.0,
            recenter: Button::R3,
            invert: (false, false),
        }
    }
}

pub struct Service {
    pub shared: Arc<Mutex<Option<PortInput>>>,
    pub rumble: Arc<AtomicBool>,
    pub motion: Arc<Mutex<MotionSettings>>,
    pub device: Arc<Mutex<Option<String>>>,
}

static SERVICE: OnceLock<Service> = OnceLock::new();

pub fn service() -> &'static Service {
    SERVICE.get_or_init(|| {
        let shared = Arc::new(Mutex::new(None));
        let rumble = Arc::new(AtomicBool::new(false));
        let motion = Arc::new(Mutex::new(MotionSettings::default()));
        let device = Arc::new(Mutex::new(None));

        let thread_shared = shared.clone();
        let thread_rumble = rumble.clone();
        let thread_motion = motion.clone();
        let thread_device = device.clone();
        std::thread::Builder::new()
            .name("hostinput-sdl".into())
            .spawn(move || self::run(thread_shared, thread_rumble, thread_motion, thread_device))
            .expect("failed to spawn SDL input thread");

        Service {
            shared,
            rumble,
            motion,
            device,
        }
    })
}

struct Slot {
    pad: Gamepad,
    id: u32,
    caps: Capabilities,
    pointer: GyroPointer,
    accel: [f32; 3],
    touch: Option<(f32, f32)>,
    last_gyro_ts: Option<u64>,
    rumble_on: bool,
    rumble_refresh: Instant,
}

fn run(
    shared: Arc<Mutex<Option<PortInput>>>,
    rumble: Arc<AtomicBool>,
    motion: Arc<Mutex<MotionSettings>>,
    device: Arc<Mutex<Option<String>>>,
) {
    let sdl = match sdl3::init() {
        Ok(sdl) => sdl,
        Err(err) => {
            tracing::error!(%err, "SDL init failed, gamepads unavailable");
            return;
        }
    };

    let subsystem = match sdl.gamepad() {
        Ok(subsystem) => subsystem,
        Err(err) => {
            tracing::error!(%err, "SDL gamepad subsystem unavailable");
            return;
        }
    };

    let mut pump = match sdl.event_pump() {
        Ok(pump) => pump,
        Err(err) => {
            tracing::error!(%err, "SDL event pump unavailable");
            return;
        }
    };

    let mut slot: Option<Slot> = None;

    if let Ok(ids) = subsystem.gamepads() {
        for id in ids {
            self::add_device(&subsystem, &mut slot, &device, id);
        }
    }

    loop {
        let settings = *motion.lock().unwrap();

        let first = pump.wait_event_timeout(Duration::from_millis(4));

        for event in first.into_iter().chain(std::iter::from_fn(|| pump.poll_event())) {
            self::handle_event(&subsystem, &mut slot, &settings, &device, event);
        }

        let snapshot = slot.as_mut().map(|slot| {
            let mut state = self::read_state(&slot.pad);
            state.accel = slot.accel;

            if state.pressed(settings.recenter) {
                slot.pointer.recenter();
            }

            PortInput {
                state,
                caps: slot.caps,
                pointer: slot.caps.gyro.then(|| slot.pointer.ir()).flatten(),
                touch: slot.touch,
            }
        });

        *shared.lock().unwrap() = snapshot;

        if let Some(slot) = slot.as_mut()
            && slot.caps.rumble
        {
            let desired = rumble.load(Ordering::Relaxed);
            let refresh = desired && slot.rumble_refresh.elapsed() >= Duration::from_millis(200);

            if desired != slot.rumble_on || refresh {
                let (low, high) = if desired { (0xC000, 0xC000) } else { (0, 0) };
                if let Err(err) = slot.pad.set_rumble(low, high, 500) {
                    tracing::debug!(id = slot.id, %err, "rumble update failed");
                }

                slot.rumble_on = desired;
                slot.rumble_refresh = Instant::now();
            }
        }
    }
}

fn handle_event(
    subsystem: &GamepadSubsystem,
    slot: &mut Option<Slot>,
    settings: &MotionSettings,
    device: &Mutex<Option<String>>,
    event: Event,
) {
    match event {
        Event::ControllerDeviceAdded { which, .. } => {
            self::add_device(subsystem, slot, device, sdl3::sys::joystick::SDL_JoystickID(which))
        }
        Event::ControllerDeviceRemoved { which, .. } => self::remove_device(subsystem, slot, device, which),
        Event::ControllerSensorUpdated {
            timestamp,
            which,
            sensor,
            data,
        } => self::sensor_update(slot, settings, which, sensor, data, timestamp),
        Event::ControllerTouchpadDown {
            which, finger, x, y, ..
        }
        | Event::ControllerTouchpadMotion {
            which, finger, x, y, ..
        } => self::touch_update(slot, which, finger, Some((x, y))),
        Event::ControllerTouchpadUp { which, finger, .. } => self::touch_update(slot, which, finger, None),
        _ => {}
    }
}

fn touch_update(slot: &mut Option<Slot>, id: u32, finger: i32, touch: Option<(f32, f32)>) {
    if finger != 0 {
        return;
    }

    if let Some(slot) = slot.as_mut().filter(|slot| slot.id == id) {
        slot.touch = touch;
    }
}

fn sensor_update(
    slot: &mut Option<Slot>,
    settings: &MotionSettings,
    id: u32,
    sensor: SensorType,
    data: [f32; 3],
    timestamp: u64,
) {
    let Some(slot) = slot.as_mut().filter(|slot| slot.id == id) else {
        return;
    };

    match sensor {
        SensorType::Gyroscope => {
            let dt = match slot.last_gyro_ts {
                Some(prev) if timestamp > prev => ((timestamp - prev) as f32 / 1_000_000_000.0).min(0.05),
                _ => 0.0,
            };
            slot.last_gyro_ts = Some(timestamp);

            slot.pointer.integrate(data, dt, settings.sensitivity, settings.invert);
        }
        SensorType::Accelerometer => {
            slot.accel = [
                data[0] / STANDARD_GRAVITY,
                data[1] / STANDARD_GRAVITY,
                data[2] / STANDARD_GRAVITY,
            ];
        }
        _ => {}
    }
}

fn add_device(subsystem: &GamepadSubsystem, slot: &mut Option<Slot>, device: &Mutex<Option<String>>, id: JoystickId) {
    if slot.as_ref().is_some_and(|slot| slot.id == u32::from(id)) {
        return;
    }

    let pad = match subsystem.open(id) {
        Ok(pad) => pad,
        Err(err) => {
            tracing::warn!(id = u32::from(id), %err, "failed to open gamepad");
            return;
        }
    };

    let id = u32::from(id);
    let caps = self::query_caps(&pad);

    let replace = match slot.as_ref() {
        None => true,
        Some(current) => caps.gyro && !current.caps.gyro,
    };

    if !replace {
        tracing::info!(id, name = ?pad.name(), "gamepad ignored, slot occupied");
        return;
    }

    tracing::info!(id, name = ?pad.name(), ?caps, "gamepad connected");

    *device.lock().unwrap() = pad.name();

    if caps.gyro
        && let Err(err) = pad.sensor_set_enabled(SensorType::Gyroscope, true)
    {
        tracing::warn!(id, %err, "failed to enable gyroscope");
    }
    if caps.accel
        && let Err(err) = pad.sensor_set_enabled(SensorType::Accelerometer, true)
    {
        tracing::warn!(id, %err, "failed to enable accelerometer");
    }

    *slot = Some(Slot {
        pad,
        id,
        caps,
        pointer: GyroPointer::default(),
        accel: [0.0; 3],
        touch: None,
        last_gyro_ts: None,
        rumble_on: false,
        rumble_refresh: Instant::now(),
    });
}

fn remove_device(subsystem: &GamepadSubsystem, slot: &mut Option<Slot>, device: &Mutex<Option<String>>, id: u32) {
    if !slot.as_ref().is_some_and(|slot| slot.id == id) {
        return;
    }

    tracing::info!(id, "gamepad disconnected");
    *slot = None;
    *device.lock().unwrap() = None;

    if let Ok(ids) = subsystem.gamepads() {
        for id in ids {
            self::add_device(subsystem, slot, device, id);
        }
    }
}

fn query_caps(pad: &Gamepad) -> Capabilities {
    Capabilities {
        analog_sticks: pad.has_axis(Axis::LeftX),
        gyro: unsafe { pad.has_sensor(SensorType::Gyroscope) },
        accel: unsafe { pad.has_sensor(SensorType::Accelerometer) },
        rumble: unsafe { pad.has_rumble() },
    }
}

const BUTTON_MAP: [(SdlButton, Button); 16] = [
    (SdlButton::South, Button::South),
    (SdlButton::East, Button::East),
    (SdlButton::West, Button::West),
    (SdlButton::North, Button::North),
    (SdlButton::LeftShoulder, Button::L1),
    (SdlButton::RightShoulder, Button::R1),
    (SdlButton::LeftStick, Button::L3),
    (SdlButton::RightStick, Button::R3),
    (SdlButton::Start, Button::Start),
    (SdlButton::Back, Button::Select),
    (SdlButton::Guide, Button::Guide),
    (SdlButton::DPadUp, Button::DpadUp),
    (SdlButton::DPadDown, Button::DpadDown),
    (SdlButton::DPadLeft, Button::DpadLeft),
    (SdlButton::DPadRight, Button::DpadRight),
    (SdlButton::Touchpad, Button::TouchpadClick),
];

fn read_state(pad: &Gamepad) -> DeviceState {
    let mut buttons = 0u32;
    for (sdl, button) in BUTTON_MAP {
        if pad.button(sdl) {
            buttons |= button.mask();
        }
    }

    let l2 = self::trigger(pad, Axis::TriggerLeft);
    let r2 = self::trigger(pad, Axis::TriggerRight);

    if l2 >= 0.5 {
        buttons |= Button::L2.mask();
    }
    if r2 >= 0.5 {
        buttons |= Button::R2.mask();
    }

    DeviceState {
        buttons,
        left: (self::axis(pad, Axis::LeftX), -self::axis(pad, Axis::LeftY)),
        right: (self::axis(pad, Axis::RightX), -self::axis(pad, Axis::RightY)),
        l2,
        r2,
        accel: [0.0; 3],
    }
}

fn axis(pad: &Gamepad, axis: Axis) -> f32 {
    (pad.axis(axis) as f32 / 32767.0).clamp(-1.0, 1.0)
}

fn trigger(pad: &Gamepad, axis: Axis) -> f32 {
    (pad.axis(axis) as f32 / 32767.0).clamp(0.0, 1.0)
}
