pub struct DspStack<const N: usize> {
    data: [u16; N],
    ptr: u8,
}

impl<const N: usize> Default for DspStack<N> {
    fn default() -> Self {
        Self { data: [0; N], ptr: 0 }
    }
}

impl<const N: usize> DspStack<N> {
    const MASK: u8 = (N - 1) as u8;

    #[inline(always)]
    pub fn top(&self) -> u16 {
        debug_assert!((self.ptr as usize) < N);
        unsafe { *self.data.get_unchecked(self.ptr as usize) }
    }

    #[inline(always)]
    pub fn set_top(&mut self, value: u16) {
        debug_assert!((self.ptr as usize) < N);
        unsafe { *self.data.get_unchecked_mut(self.ptr as usize) = value }
    }

    #[inline(always)]
    pub fn push(&mut self, value: u16) {
        self.ptr = (self.ptr + 1) & Self::MASK;
        debug_assert!((self.ptr as usize) < N);
        unsafe { *self.data.get_unchecked_mut(self.ptr as usize) = value }
    }

    #[inline(always)]
    pub fn pop(&mut self) -> u16 {
        debug_assert!((self.ptr as usize) < N);
        let value = unsafe { *self.data.get_unchecked(self.ptr as usize) };
        self.ptr = (self.ptr.wrapping_sub(1)) & Self::MASK;
        value
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.ptr == 0
    }
}
