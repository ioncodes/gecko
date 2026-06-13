pub trait ExiDevice: Send {
    fn on_select(&mut self) {}
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

    fn connected(&self) -> bool {
        true
    }

    fn is_stub(&self) -> bool {
        false
    }
}

pub struct ExiDummy;

impl ExiDevice for ExiDummy {
    fn transfer_byte(&mut self, byte: &mut u8) {
        *byte = 0;
    }

    fn connected(&self) -> bool {
        false
    }

    fn is_stub(&self) -> bool {
        true
    }
}
