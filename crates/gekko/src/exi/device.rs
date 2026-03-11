pub trait ExiDevice {
    fn transfer_byte(&mut self, byte: &mut u8);

    fn dma_read(&mut self, buf: &mut [u8]) {
        for b in buf.iter_mut() {
            *b = 0;
            self.transfer_byte(b);
        }
    }

    fn dma_write(&mut self, buf: &[u8]) {
        for b in buf {
            let mut b = *b;
            self.transfer_byte(&mut b);
        }
    }
}

pub struct ExiDummy;

impl ExiDevice for ExiDummy {
    fn transfer_byte(&mut self, byte: &mut u8) {
        *byte = 0;
    }
}
