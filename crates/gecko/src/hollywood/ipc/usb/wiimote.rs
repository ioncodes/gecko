use super::crypto::Cipher;

pub const BTN_TWO: u16 = 0x0001;
pub const BTN_ONE: u16 = 0x0002;
pub const BTN_B: u16 = 0x0004;
pub const BTN_A: u16 = 0x0008;
pub const BTN_MINUS: u16 = 0x0010;
pub const BTN_HOME: u16 = 0x0080;
pub const BTN_LEFT: u16 = 0x0100;
pub const BTN_RIGHT: u16 = 0x0200;
pub const BTN_DOWN: u16 = 0x0400;
pub const BTN_UP: u16 = 0x0800;
pub const BTN_PLUS: u16 = 0x1000;

pub const NUNCHUK_BTN_C: u8 = 0x02;
pub const NUNCHUK_BTN_Z: u8 = 0x01;

pub const NUNCHUK_STICK_MIN: u8 = 0x00;
pub const NUNCHUK_STICK_CENTER: u8 = 0x80;
pub const NUNCHUK_STICK_MAX: u8 = 0xFF;

pub const IR_CAMERA_WIDTH: u16 = 1024;
pub const IR_CAMERA_HEIGHT: u16 = 768;

const HID_PREFIX_INPUT: u8 = 0xA1;
const HID_PREFIX_OUTPUT: u8 = 0xA2;

/// Output reports the host writes on the HID interrupt channel.
/// Values per <https://wiibrew.org/wiki/Wiimote#Output_Reports>.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum OutputReportId {
    Rumble = 0x10,
    PlayerLeds = 0x11,
    DataReportingMode = 0x12,
    IrCameraEnable = 0x13,
    SpeakerEnable = 0x14,
    StatusInformationRequest = 0x15,
    WriteMemoryAndRegisters = 0x16,
    ReadMemoryAndRegisters = 0x17,
    SpeakerData = 0x18,
    SpeakerMute = 0x19,
    IrCameraEnable2 = 0x1A,
}

impl OutputReportId {
    fn from_u8(id: u8) -> Option<Self> {
        Some(match id {
            0x10 => Self::Rumble,
            0x11 => Self::PlayerLeds,
            0x12 => Self::DataReportingMode,
            0x13 => Self::IrCameraEnable,
            0x14 => Self::SpeakerEnable,
            0x15 => Self::StatusInformationRequest,
            0x16 => Self::WriteMemoryAndRegisters,
            0x17 => Self::ReadMemoryAndRegisters,
            0x18 => Self::SpeakerData,
            0x19 => Self::SpeakerMute,
            0x1A => Self::IrCameraEnable2,
            _ => return None,
        })
    }
}

/// Data reporting modes the host selects via `DataReportingMode (0x12)`.
/// Values and layouts per <https://wiibrew.org/wiki/Wiimote#Data_Reporting>.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum ReportMode {
    /// Core buttons only.
    Core = 0x30,
    /// Core + 3-byte accelerometer.
    CoreAccel = 0x31,
    /// Core + 8-byte extension.
    CoreExt8 = 0x32,
    /// Core + accel + 12-byte IR.
    CoreAccelIrExt = 0x33,
    /// Core + 19-byte extension.
    CoreExt19 = 0x34,
    /// Core + accel + 16-byte extension.
    CoreAccelExt16 = 0x35,
    /// Core + 10-byte basic IR + 9-byte extension (no accel).
    CoreIrBasicExt9 = 0x36,
    /// Core + accel + 10-byte basic IR + 6-byte extension.
    CoreAccelIrBasicExt6 = 0x37,
}

impl ReportMode {
    fn from_u8(mode: u8) -> Option<Self> {
        Some(match mode {
            0x30 => Self::Core,
            0x31 => Self::CoreAccel,
            0x32 => Self::CoreExt8,
            0x33 => Self::CoreAccelIrExt,
            0x34 => Self::CoreExt19,
            0x35 => Self::CoreAccelExt16,
            0x36 => Self::CoreIrBasicExt9,
            0x37 => Self::CoreAccelIrBasicExt6,
            _ => return None,
        })
    }
}

const WIIMOTE_EEPROM_SIZE: usize = 0x1700;
const WIIMOTE_EEPROM_CALIBRATION: [u8; 42] = [
    // IR sensor calibration #1 (0x00..0x0B): 4 reference dot positions + checksum
    0x7F, 0x5D, 0x03, 0x80, 0x5D, 0x80, 0xA2, 0xB8, 0x7F, 0xA2, 0x0C,
    // IR sensor calibration #2 (0x0B..0x16): duplicate
    0x7F, 0x5D, 0x03, 0x80, 0x5D, 0x80, 0xA2, 0xB8, 0x7F, 0xA2, 0x0C, // Accel calibration #1 (0x16..0x20)
    0x80, 0x80, 0x80, 0x00, // accel zero G (X, Y, Z, padding)
    0x9A, 0x9A, 0x9A, 0x00, // accel one G (X, Y, Z, padding)
    0x00, 0xA3, // padding, checksum
    // Accel calibration #2 (0x20..0x2A): duplicate
    0x80, 0x80, 0x80, 0x00, 0x9A, 0x9A, 0x9A, 0x00, 0x00, 0xA3,
];

const NUNCHUK_ID: [u8; 6] = [0x00, 0x00, 0xA4, 0x20, 0x00, 0x00];
const NUNCHUK_CALIBRATION: [u8; 16] = [
    0x80, 0x80, 0x80, 0x00, // accel zero G (X, Y, Z, padding)
    0xB3, 0xB3, 0xB3, 0x00, // accel one G (X, Y, Z, padding)
    0xE0, 0x20, 0x80, // stick X max, min, center
    0xE0, 0x20, 0x80, // stick Y max, min, center
    0x00, 0x00, // checksum (filled below at runtime)
];

const NUNCHUK_KEY_REG_BASE: u8 = 0x40;
const NUNCHUK_KEY_LEN: usize = 16;

const WIIMOTE_ACCEL_ZERO_G: i32 = 0x80;
const WIIMOTE_ACCEL_LSB_PER_G: f32 = 0x1A as f32;
const ACCEL_REST: [u8; 3] = [0x80, 0x80, 0xB3];

const SHAKE_FREQ_HZ: f32 = 10.0;
const SHAKE_AMPLITUDE_G: f32 = 2.0;
const SHAKE_PHASE_STEP: f32 = 2.0 * std::f32::consts::PI * SHAKE_FREQ_HZ / 60.0;

#[inline]
fn encode_wiimote_g(g: f32) -> u8 {
    (WIIMOTE_ACCEL_ZERO_G + (g * WIIMOTE_ACCEL_LSB_PER_G).round() as i32).clamp(0, 255) as u8
}

#[derive(Debug, Clone)]
pub(super) struct WiimoteState {
    buttons: u16,
    report_mode: ReportMode,
    continuous: bool,
    leds: u8,
    ir_enabled_pin1: bool,
    ir_enabled_pin2: bool,
    eeprom: Vec<u8>,
    nunchuk_attached: bool,
    nunchuk_buttons: u8,
    nunchuk_stick_x: u8,
    nunchuk_stick_y: u8,
    nunchuk_calibration: [u8; 16],
    nunchuk_key_buf: [u8; NUNCHUK_KEY_LEN],
    nunchuk_key_valid: u16,
    nunchuk_cipher: Cipher,
    ir_pointer: Option<(u16, u16)>,
    accel: [u8; 3],
    shake_phase: f32,
}

impl Default for WiimoteState {
    fn default() -> Self {
        let mut eeprom = vec![0u8; WIIMOTE_EEPROM_SIZE];
        eeprom[0..WIIMOTE_EEPROM_CALIBRATION.len()].copy_from_slice(&WIIMOTE_EEPROM_CALIBRATION);

        let mut nunchuk_calibration = NUNCHUK_CALIBRATION;
        let (a, b) = self::compute_calibration_checksum(&nunchuk_calibration[..14]);
        nunchuk_calibration[14] = a;
        nunchuk_calibration[15] = b;

        Self {
            buttons: 0,
            report_mode: ReportMode::Core,
            continuous: false,
            leds: 0,
            ir_enabled_pin1: false,
            ir_enabled_pin2: false,
            eeprom,
            nunchuk_attached: true,
            nunchuk_buttons: 0,
            nunchuk_stick_x: NUNCHUK_STICK_CENTER,
            nunchuk_stick_y: NUNCHUK_STICK_CENTER,
            nunchuk_calibration,
            nunchuk_key_buf: [0; NUNCHUK_KEY_LEN],
            nunchuk_key_valid: 0,
            nunchuk_cipher: Cipher::IDENTITY,
            ir_pointer: None,
            accel: ACCEL_REST,
            shake_phase: 0.0,
        }
    }
}

impl WiimoteState {
    pub(super) fn set_buttons(&mut self, buttons: u16) -> bool {
        let old = self.buttons;
        self.buttons = buttons;
        self.buttons != old
    }

    pub(super) fn set_nunchuk(&mut self, buttons: u8, stick_x: u8, stick_y: u8) -> bool {
        let changed =
            self.nunchuk_buttons != buttons || self.nunchuk_stick_x != stick_x || self.nunchuk_stick_y != stick_y;
        self.nunchuk_buttons = buttons;
        self.nunchuk_stick_x = stick_x;
        self.nunchuk_stick_y = stick_y;
        changed
    }

    pub(super) fn tick_shake(&mut self, active: bool) -> bool {
        if active {
            self.shake_phase = (self.shake_phase + SHAKE_PHASE_STEP) % (2.0 * std::f32::consts::PI);
            let lol = SHAKE_AMPLITUDE_G * self.shake_phase.sin();
            self.accel = [
                encode_wiimote_g(lol),
                encode_wiimote_g(lol),
                encode_wiimote_g(1.0 + lol),
            ];
            true
        } else {
            self.shake_phase = 0.0;
            let was_at_rest = self.accel == ACCEL_REST;
            self.accel = ACCEL_REST;
            !was_at_rest
        }
    }

    pub(super) fn set_ir_pointer(&mut self, pointer: Option<(u16, u16)>) -> bool {
        let changed = self.ir_pointer != pointer;
        self.ir_pointer = pointer;
        changed
    }

    fn button_bytes(&self) -> [u8; 2] {
        self.buttons.to_be_bytes()
    }

    pub(super) fn make_input_report(&self) -> Vec<u8> {
        let [bb0, bb1] = self.button_bytes();
        let accel = self.accel;
        let mode = self.report_mode;

        let mut r = vec![HID_PREFIX_INPUT, mode as u8, bb0, bb1];
        match mode {
            ReportMode::Core => {}
            ReportMode::CoreAccel => r.extend_from_slice(&accel),
            ReportMode::CoreExt8 => self.append_nunchuk_ext_padded(&mut r, 8),
            ReportMode::CoreAccelIrExt => {
                r.extend_from_slice(&accel);
                r.extend_from_slice(&self::pack_ir_extended(&self.ir_objects()));
            }
            ReportMode::CoreExt19 => self.append_nunchuk_ext_padded(&mut r, 19),
            ReportMode::CoreAccelExt16 => {
                r.extend_from_slice(&accel);
                self.append_nunchuk_ext_padded(&mut r, 16);
            }
            ReportMode::CoreIrBasicExt9 => {
                r.extend_from_slice(&self::pack_ir_basic(&self.ir_objects()));
                self.append_nunchuk_ext_padded(&mut r, 9);
            }
            ReportMode::CoreAccelIrBasicExt6 => {
                r.extend_from_slice(&accel);
                r.extend_from_slice(&self::pack_ir_basic(&self.ir_objects()));
                r.extend_from_slice(&self.nunchuk_extension_bytes());
            }
        }
        r
    }

    /// 6 extension bytes encoding the nunchuk's stick + accel + button state.
    /// Per <https://wiibrew.org/wiki/Wiimote/Extension_Controllers/Nunchuk>.
    fn nunchuk_extension_bytes(&self) -> [u8; 6] {
        if !self.nunchuk_attached {
            return [0xFF; 6];
        }

        // Bits 0,1 of the trailing byte are inverted: 1 = button NOT pressed.
        let inv_buttons = (!self.nunchuk_buttons) & 0x03;
        let mut bytes = [
            self.nunchuk_stick_x,
            self.nunchuk_stick_y,
            0x80, // accel X high bits (zero G)
            0x80, // accel Y high bits (zero G)
            0xB3, // accel Z high bits (+1G gravity)
            inv_buttons,
        ];
        self.nunchuk_cipher.encrypt(&mut bytes, 0);
        bytes
    }

    /// Append the 6-byte nunchuk extension followed by 0xFF padding so the
    /// report's extension slot is exactly `total_len` bytes.
    fn append_nunchuk_ext_padded(&self, out: &mut Vec<u8>, total_len: usize) {
        let target = out.len() + total_len;
        out.extend_from_slice(&self.nunchuk_extension_bytes());
        out.resize(target, 0xFF);
    }

    fn ir_objects(&self) -> [Option<(u16, u16)>; 4] {
        const SENSOR_BAR_HALF: u16 = 50;

        let Some((cx, cy)) = self.ir_pointer else {
            return [None; 4];
        };

        let left = cx.saturating_sub(SENSOR_BAR_HALF);
        let right = (cx + SENSOR_BAR_HALF).min(IR_CAMERA_WIDTH - 1);

        [Some((left, cy)), Some((right, cy)), None, None]
    }

    pub(super) fn handle_output_report(&mut self, packet: &[u8]) -> Vec<Vec<u8>> {
        if packet.len() < 2 {
            return Vec::new();
        }

        if packet[0] != HID_PREFIX_OUTPUT {
            tracing::warn!(
                packet = format!("{packet:02X?}"),
                "unexpected Wiimote output packet prefix"
            );
            return Vec::new();
        }

        let raw_id = packet[1];
        let body = &packet[2..];

        let Some(report_id) = OutputReportId::from_u8(raw_id) else {
            tracing::warn!(report_id = format!("{raw_id:#04x}"), "ignored Wiimote output report");
            return Vec::new();
        };

        tracing::debug!(report_id = ?report_id, "received Wiimote output report");

        match report_id {
            OutputReportId::Rumble
            | OutputReportId::SpeakerEnable
            | OutputReportId::SpeakerData
            | OutputReportId::SpeakerMute => self::trivial_ack(self.button_bytes(), raw_id),
            OutputReportId::PlayerLeds => {
                self.leds = body.first().copied().unwrap_or(0) >> 4;
                self::trivial_ack(self.button_bytes(), raw_id)
            }
            OutputReportId::DataReportingMode if body.len() >= 2 => {
                self.continuous = (body[0] & 0x04) != 0;
                let Some(mode) = ReportMode::from_u8(body[1]) else {
                    tracing::warn!(
                        report_mode = format!("{:#04x}", body[1]),
                        "unsupported Wiimote report mode, ignoring"
                    );
                    return self::trivial_ack(self.button_bytes(), raw_id);
                };
                self.report_mode = mode;

                tracing::debug!(
                    report_mode = ?mode,
                    continuous = self.continuous,
                    "Wiimote data reporting mode selected"
                );

                self::trivial_ack(self.button_bytes(), raw_id)
            }
            OutputReportId::IrCameraEnable => {
                self.ir_enabled_pin1 = body.first().is_some_and(|&b| (b & 0x04) != 0);
                self::trivial_ack(self.button_bytes(), raw_id)
            }
            OutputReportId::StatusInformationRequest => {
                tracing::debug!("Wiimote status requested");
                vec![self.make_status_report()]
            }
            OutputReportId::WriteMemoryAndRegisters if body.len() >= 5 => {
                let address = self::decode_mem_address(body);

                let address_space = body[0] & 0x06;
                let size = body[4] as usize;
                let payload = &body[5..5 + size.min(body.len().saturating_sub(5)).min(16)];

                tracing::debug!(addr = format!("{address:#08x}"), "Wiimote write memory");

                if address_space == 0 {
                    let end = (address as usize + payload.len()).min(self.eeprom.len());
                    let dst = &mut self.eeprom[address as usize..end];
                    dst.copy_from_slice(&payload[..dst.len()]);
                } else {
                    self.observe_register_write(address, payload);
                }

                self::trivial_ack(self.button_bytes(), raw_id)
            }
            OutputReportId::ReadMemoryAndRegisters if body.len() >= 6 => {
                let address_space = body[0] & 0x06;
                let address = self::decode_mem_address(body);
                let size = u16::from_be_bytes([body[4], body[5]]) as usize;

                tracing::debug!(
                    addr = format!("{address:#08x}"),
                    size,
                    space = address_space,
                    "Wiimote read memory"
                );

                self.read_memory_response(address_space, address, size)
            }
            OutputReportId::IrCameraEnable2 => {
                self.ir_enabled_pin2 = body.first().is_some_and(|&b| (b & 0x04) != 0);
                self::trivial_ack(self.button_bytes(), raw_id)
            }
            // Body too short for the variant's required fields.
            _ => Vec::new(),
        }
    }

    fn read_memory_response(&self, address_space: u8, address: u32, size: usize) -> Vec<Vec<u8>> {
        if address_space != 0 {
            return self.read_register_response(address, size);
        }

        let start = address as usize;
        let end = start + size;
        if end > self.eeprom.len() {
            return vec![self.read_chunk_report(address as u16, &[], 0x08)];
        }

        let mut out = Vec::new();
        let mut offset = 0usize;
        while offset < size {
            let chunk = (size - offset).min(16);
            let chunk_addr = address as u16 + offset as u16;
            out.push(self.read_chunk_report(chunk_addr, &self.eeprom[start + offset..start + offset + chunk], 0));

            offset += chunk;
        }

        out
    }

    fn read_register_response(&self, address: u32, size: usize) -> Vec<Vec<u8>> {
        let canonical = address & 0x00FFFFFF;

        if !self.nunchuk_attached {
            tracing::debug!(
                addr = format!("{address:#08x}"),
                "register read with no extension; returning no-peripheral error"
            );

            return vec![self.read_chunk_report(address as u16, &[], 0x07)];
        }

        let mut backing = [0u8; 0x100];
        backing[0x20..0x30].copy_from_slice(&self.nunchuk_calibration);
        backing[0xFA..0x100].copy_from_slice(&NUNCHUK_ID);

        let base = (canonical & 0xFF) as usize;

        let mut out = Vec::new();
        let mut offset = 0usize;
        while offset < size {
            let chunk = (size - offset).min(16);
            let start = base + offset;
            let end = start + chunk;

            if end > backing.len() {
                out.push(self.read_chunk_report((address as u16) + offset as u16, &[], 0x08));
                break;
            }

            // The SDK runs `kpad_extension_crypt_buffer` (0x803DA000) over the
            // returned bytes when the address sits in extension register space
            // (see `kpad_dispatch_report` 0x803D66D0 read handler branch on
            // `v13 == 1188`, i.e. high word == 0x4A4). It uses the low 16 bits
            // of the request address as the cipher offset.
            let chunk_addr = (address as u16) + offset as u16;
            let mut chunk_data: [u8; 16] = [0; 16];
            chunk_data[..chunk].copy_from_slice(&backing[start..end]);
            self.nunchuk_cipher.encrypt(&mut chunk_data[..chunk], chunk_addr as u32);

            out.push(self.read_chunk_report(chunk_addr, &chunk_data[..chunk], 0));

            offset += chunk;
        }
        out
    }

    /// Track writes to nunchuk register 0x40-0x4F so we can mirror the cipher
    /// the SDK builds in `kpad_nunchuk_keygen` (0x803D9210). Once all 16 bytes
    /// have arrived we derive `ft`/`sb` and switch our outgoing extension
    /// bytes and register read responses to encrypted output.
    fn observe_register_write(&mut self, address: u32, payload: &[u8]) {
        let reg = (address & 0xFF) as u8;
        let Some(rel) = reg.checked_sub(NUNCHUK_KEY_REG_BASE) else {
            return;
        };

        let rel = rel as usize;
        if rel >= NUNCHUK_KEY_LEN {
            return;
        }

        let count = payload.len().min(NUNCHUK_KEY_LEN - rel);
        self.nunchuk_key_buf[rel..rel + count].copy_from_slice(&payload[..count]);
        for i in 0..count {
            self.nunchuk_key_valid |= 1 << (rel + i);
        }

        if self.nunchuk_key_valid == u16::MAX {
            self.nunchuk_cipher = Cipher::from_extension_key(&self.nunchuk_key_buf);
            tracing::debug!(
                key = format!("{:02X?}", self.nunchuk_key_buf),
                "Wiimote nunchuk encryption key complete"
            );
        }
    }

    fn read_chunk_report(&self, address: u16, data: &[u8], error: u8) -> Vec<u8> {
        let [bb0, bb1] = self.button_bytes();
        let chunk_len = data.len().min(16);
        let size_and_error = if chunk_len == 0 {
            error & 0x0F
        } else {
            (((chunk_len as u8 - 1) & 0x0F) << 4) | (error & 0x0F)
        };

        let mut report = Vec::with_capacity(22);
        report.extend_from_slice(&[HID_PREFIX_INPUT, 0x21, bb0, bb1, size_and_error]);
        report.extend_from_slice(&address.to_be_bytes());
        report.extend_from_slice(&data[..chunk_len]);

        for _ in chunk_len..16 {
            report.push(0);
        }

        report
    }

    fn make_status_report(&self) -> Vec<u8> {
        let [bb0, bb1] = self.button_bytes();
        let ir_enabled = self.ir_enabled_pin1 && self.ir_enabled_pin2;
        let mut flags = self.leds << 4;

        if ir_enabled {
            flags |= 0x08;
        }

        if self.nunchuk_attached {
            flags |= 0x02; // extension connected
        }

        vec![HID_PREFIX_INPUT, 0x20, bb0, bb1, flags, 0x00, 0x00, 0x64]
    }
}

fn compute_calibration_checksum(data: &[u8]) -> (u8, u8) {
    // Per wiibrew: first byte = sum + 0x55, second byte = sum + 0xAA.
    let mut sum: u8 = 0;
    for &b in data {
        sum = sum.wrapping_add(b);
    }
    (sum.wrapping_add(0x55), sum.wrapping_add(0xAA))
}

fn pack_ir_basic(objects: &[Option<(u16, u16)>; 4]) -> [u8; 10] {
    let mut out = [0xFF; 10];

    self::pack_ir_basic_pair(&mut out[0..5], objects[0], objects[1]);
    self::pack_ir_basic_pair(&mut out[5..10], objects[2], objects[3]);

    out
}

fn split_ir_coords(x: u16, y: u16) -> (u8, u8, u8, u8) {
    (x as u8, ((x >> 8) & 0x03) as u8, y as u8, ((y >> 8) & 0x03) as u8)
}

fn pack_ir_basic_pair(out: &mut [u8], p1: Option<(u16, u16)>, p2: Option<(u16, u16)>) {
    if let Some((x, y)) = p1 {
        let (x_lo, x_hi, y_lo, y_hi) = self::split_ir_coords(x, y);
        out[0] = x_lo;
        out[1] = y_lo;
        out[2] = (out[2] & 0xF0) | (y_hi << 2) | x_hi;
    }

    if let Some((x, y)) = p2 {
        let (x_lo, x_hi, y_lo, y_hi) = self::split_ir_coords(x, y);
        out[3] = x_lo;
        out[4] = y_lo;
        out[2] = (out[2] & 0x0F) | (y_hi << 6) | (x_hi << 4);
    }
}

fn pack_ir_extended(objects: &[Option<(u16, u16)>; 4]) -> [u8; 12] {
    let mut out = [0xFF; 12];

    for (i, slot) in objects.iter().enumerate() {
        if let Some((x, y)) = *slot {
            let base = i * 3;
            let (x_lo, x_hi, y_lo, y_hi) = self::split_ir_coords(x, y);
            out[base] = x_lo;
            out[base + 1] = y_lo;
            out[base + 2] = (3 << 4) | (y_hi << 2) | x_hi;
        }
    }

    out
}

#[inline(always)]
fn decode_mem_address(body: &[u8]) -> u32 {
    ((body[1] as u32) << 16) | ((body[2] as u32) << 8) | (body[3] as u32)
}

#[inline(always)]
fn trivial_ack(button_bytes: [u8; 2], report_id: u8) -> Vec<Vec<u8>> {
    vec![vec![
        HID_PREFIX_INPUT,
        0x22,
        button_bytes[0],
        button_bytes[1],
        report_id,
        0x00,
    ]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_button_report_encodes_a_press_and_release() {
        let mut wiimote = WiimoteState::default();

        assert!(wiimote.set_buttons(BTN_A));
        assert_eq!(wiimote.make_input_report(), [0xA1, 0x30, 0x00, 0x08]);

        assert!(wiimote.set_buttons(0));
        assert_eq!(wiimote.make_input_report(), [0xA1, 0x30, 0x00, 0x00]);
    }

    #[test]
    fn data_reporting_mode_selects_extended_minimal_reports() {
        let mut wiimote = WiimoteState::default();

        assert!(wiimote.set_buttons(BTN_A));
        let acks = wiimote.handle_output_report(&[0xA2, 0x12, 0x04, 0x31]);
        assert_eq!(acks, vec![vec![0xA1, 0x22, 0x00, 0x08, 0x12, 0x00]]);
        assert!(wiimote.continuous);
        assert_eq!(wiimote.make_input_report(), [0xA1, 0x31, 0x00, 0x08, 0x80, 0x80, 0xB3]);

        let acks = wiimote.handle_output_report(&[0xA2, 0x12, 0x00, 0x33]);
        assert_eq!(acks, vec![vec![0xA1, 0x22, 0x00, 0x08, 0x12, 0x00]]);
        let mut expected = vec![0xA1, 0x33, 0x00, 0x08, 0x80, 0x80, 0xB3];
        expected.extend_from_slice(&[0xFF; 12]);
        assert_eq!(wiimote.make_input_report(), expected);
    }

    #[test]
    fn read_memory_returns_chunked_calibration_data() {
        let mut wiimote = WiimoteState::default();
        let reports = wiimote.handle_output_report(&[0xA2, 0x17, 0x00, 0x00, 0x00, 0x16, 0x00, 0x10]);
        assert_eq!(reports.len(), 1);
        let r = &reports[0];
        assert_eq!(&r[0..2], &[0xA1, 0x21]);
        assert_eq!(r[4], 0xF0); // 16 bytes - 1 = 0x0F, shifted into upper nibble
        assert_eq!(&r[5..7], &[0x00, 0x16]);
        assert_eq!(&r[7..23], &WIIMOTE_EEPROM_CALIBRATION[22..38]);
    }

    #[test]
    fn unsupported_report_mode_is_rejected_and_kept_at_previous() {
        let mut wiimote = WiimoteState::default();
        // Pin a known mode first.
        wiimote.handle_output_report(&[0xA2, 0x12, 0x00, 0x31]);
        assert_eq!(wiimote.report_mode, ReportMode::CoreAccel);
        // Reject unknown mode 0x3D and stay on 0x31.
        wiimote.handle_output_report(&[0xA2, 0x12, 0x00, 0x3D]);
        assert_eq!(wiimote.report_mode, ReportMode::CoreAccel);
    }
}
