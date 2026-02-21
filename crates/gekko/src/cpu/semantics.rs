pub struct Instruction(pub u32);

#[rustfmt::skip]
impl Instruction {
    #[inline] pub fn primary(&self) -> u32 { self.0 >> 26 }
    #[inline] pub fn xo10(&self)    -> u32 { (self.0 >> 1) & 0x3ff }
    #[inline] pub fn xo9(&self)     -> u32 { (self.0 >> 1) & 0x1ff }
    #[inline] pub fn xo5(&self)     -> u32 { (self.0 >> 1) & 0x1f }

    #[inline] pub fn rd(&self)      -> usize { ((self.0 >> 21) & 0x1f) as usize }
    #[inline] pub fn rs(&self)      -> usize { ((self.0 >> 21) & 0x1f) as usize }
    #[inline] pub fn fd(&self)      -> usize { ((self.0 >> 21) & 0x1f) as usize }
    #[inline] pub fn fs(&self)      -> usize { ((self.0 >> 21) & 0x1f) as usize }
    #[inline] pub fn ra(&self)      -> usize { ((self.0 >> 16) & 0x1f) as usize }
    #[inline] pub fn rb(&self)      -> usize { ((self.0 >> 11) & 0x1f) as usize }
    #[inline] pub fn fc(&self)      -> usize { ((self.0 >>  6) & 0x1f) as usize }

    #[inline] pub fn rc(&self)      -> bool  { self.0 & 1 != 0 }
    #[inline] pub fn lk(&self)      -> bool  { self.0 & 1 != 0 }
    #[inline] pub fn aa(&self)      -> bool  { (self.0 >> 1) & 1 != 0 }
    #[inline] pub fn oe(&self)      -> bool  { (self.0 >> 10) & 1 != 0 }

    #[inline] pub fn simm(&self)    -> i32  { (self.0 as i16) as i32 }
    #[inline] pub fn uimm(&self)    -> u32  { self.0 & 0xffff }
    #[inline] pub fn sh(&self)      -> u32  { (self.0 >> 11) & 0x1f }
    #[inline] pub fn mb(&self)      -> u32  { (self.0 >>  6) & 0x1f }
    #[inline] pub fn me(&self)      -> u32  { (self.0 >>  1) & 0x1f }

    #[inline] pub fn li(&self)      -> i32  { (((self.0 & 0x03ff_fffc) as i32) << 6) >> 6 }
    #[inline] pub fn bd(&self)      -> i32  { (((self.0 & 0x0000_fffc) as i32) << 16) >> 16 }

    #[inline] pub fn spr(&self)     -> u32  { let raw = (self.0 >> 11) & 0x3ff; (raw >> 5) | ((raw & 0x1f) << 5) }

    #[inline] pub fn bo(&self)      -> u32  { (self.0 >> 21) & 0x1f }
    #[inline] pub fn bi(&self)      -> u32  { (self.0 >> 16) & 0x1f }
}
