use gecko::input::aim_to_ir;

const FOV_X: f32 = 0.60;
const FOV_Y: f32 = 0.45;
const OVERSHOOT: f32 = 1.2;

#[derive(Default)]
pub struct GyroPointer {
    yaw: f32,
    pitch: f32,
}

impl GyroPointer {
    pub fn integrate(&mut self, gyro: [f32; 3], dt: f32, sensitivity: f32, invert: (bool, bool)) {
        let limit_x = FOV_X * OVERSHOOT / 2.0;
        let limit_y = FOV_Y * OVERSHOOT / 2.0;

        let dx = gyro[1] * dt * sensitivity * if invert.0 { -1.0 } else { 1.0 };
        let dy = gyro[0] * dt * sensitivity * if invert.1 { -1.0 } else { 1.0 };

        self.yaw = (self.yaw - dx).clamp(-limit_x, limit_x);
        self.pitch = (self.pitch - dy).clamp(-limit_y, limit_y);
    }

    pub fn recenter(&mut self) {
        self.yaw = 0.0;
        self.pitch = 0.0;
    }

    pub fn ir(&self) -> Option<(u16, u16)> {
        if self.yaw.abs() > FOV_X / 2.0 || self.pitch.abs() > FOV_Y / 2.0 {
            return None;
        }

        Some(aim_to_ir(self.yaw / FOV_X + 0.5, self.pitch / FOV_Y + 0.5))
    }
}

pub fn map_accel(host: [f32; 3]) -> [f32; 3] {
    [-host[0], -host[2], host[1]]
}

pub fn stick_pointer(v: (f32, f32)) -> Option<(u16, u16)> {
    if v == (0.0, 0.0) {
        return None;
    }

    Some(aim_to_ir((v.0 + 1.0) / 2.0, (1.0 - v.1) / 2.0))
}
