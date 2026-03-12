use crate::gekko::Gekko;

impl Gekko {
    // Load a 64-bit double from memory
    #[inline]
    pub fn read_f64(&mut self, addr: u32) -> f64 {
        let hi = self.read_u32(addr) as u64;
        let lo = self.read_u32(addr.wrapping_add(4)) as u64;
        f64::from_bits((hi << 32) | lo)
    }

    /// Store a 64-bit double to memory
    #[inline]
    pub fn write_f64(&mut self, addr: u32, val: f64) {
        let bits = val.to_bits();
        self.write_u32(addr, (bits >> 32) as u32);
        self.write_u32(addr.wrapping_add(4), bits as u32);
    }

    /// Load a 32-bit float from memory, return as f64
    #[inline]
    pub fn read_f32(&mut self, addr: u32) -> f64 {
        f32::from_bits(self.read_u32(addr)) as f64
    }

    /// Store f64 as 32-bit float to memory
    #[inline]
    pub fn write_f32(&mut self, addr: u32, val: f64) {
        self.write_u32(addr, (val as f32).to_bits());
    }
}
