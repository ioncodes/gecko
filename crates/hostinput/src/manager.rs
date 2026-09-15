use crate::config::InputConfig;
use crate::device::{Capabilities, PortInput};
use crate::mapping::{GcProfile, PointerSource, WiiProfile};
use crate::{mapping, merge, sdl};
use gecko::{GC, HostInput, InputSink, SystemId, WII};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

pub type InputUpdates = Arc<Mutex<Option<InputConfig>>>;

pub struct InputManager {
    updates: Option<InputUpdates>,
    system: SystemId,
    keyboard: Arc<Mutex<HostInput>>,
    gc_profile: GcProfile,
    wii_profile: WiiProfile,
    service: Option<&'static sdl::Service>,
}

impl InputManager {
    pub fn new(system: SystemId, config: &InputConfig, keyboard: Arc<Mutex<HostInput>>) -> Self {
        let gc_profile = config.gc_profile();
        let wii_profile = config.wii_profile();

        let service = config.gamepads.then(sdl::service);

        let manager = Self {
            updates: None,
            system,
            keyboard,
            gc_profile,
            wii_profile,
            service,
        };
        manager.sync_motion_settings();
        manager
    }

    pub fn with_updates(mut self, updates: InputUpdates) -> Self {
        self.updates = Some(updates);
        self
    }

    fn refresh(&mut self, config: &InputConfig) {
        self.wii_profile = config.wii_profile();
        self.sync_motion_settings();
    }

    fn sync_motion_settings(&self) {
        if let Some(service) = self.service {
            *service.motion.lock().unwrap() = sdl::MotionSettings {
                sensitivity: self.wii_profile.sensitivity,
                recenter: self.wii_profile.recenter,
                invert: (self.wii_profile.pointer_invert.x, self.wii_profile.pointer_invert.y),
            };
        }
    }
}

fn resolve_pointer(profile: PointerSource, caps: &Capabilities) -> PointerSource {
    match profile {
        PointerSource::Auto if caps.gyro => PointerSource::Gyro,
        PointerSource::Auto if caps.analog_sticks => PointerSource::RightStick,
        PointerSource::Auto => PointerSource::Mouse,
        forced => forced,
    }
}

impl InputSink for InputManager {
    fn sample(&mut self) -> HostInput {
        let (kb, update) = {
            let keyboard = self.keyboard.lock().unwrap();
            let update = self.updates.as_ref().and_then(|updates| updates.lock().unwrap().take());
            (*keyboard, update)
        };

        if let Some(config) = update {
            self.refresh(&config);
        }

        let Some(service) = self.service else {
            return kb;
        };

        let port: Option<PortInput> = *service.shared.lock().unwrap();

        let Some(port) = port else {
            return kb;
        };

        match (self.system, kb) {
            (GC, HostInput::Gc(kb_pad)) => {
                HostInput::Gc(merge::gc(mapping::gc::map(&port.state, &self.gc_profile), kb_pad))
            }
            (WII, kb @ HostInput::Wii { .. }) => {
                let source = self::resolve_pointer(self.wii_profile.pointer, &port.caps);

                merge::wii(mapping::wii::map(&port, &self.wii_profile, source), kb)
            }
            _ => kb,
        }
    }

    fn set_rumble(&mut self, _port: usize, on: bool) {
        if let Some(service) = self.service {
            service.rumble.store(on, Ordering::Relaxed);
        }
    }
}
