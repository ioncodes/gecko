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

    fn on_deselect(&mut self) -> Option<u64> {
        None
    }

    fn complete_command(&mut self) -> bool {
        false
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
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

/// Load a fixed-size backing file into `dst`, leaving `dst` untouched (its
/// default contents) if the file is missing or the wrong size.
pub fn load_backing(path: &std::path::Path, dst: &mut [u8], label: &str) {
    match std::fs::read(path) {
        Ok(bytes) if bytes.len() == dst.len() => {
            dst.copy_from_slice(&bytes);
            tracing::info!(path = %path.display(), label, "loaded backing file");
        }
        Ok(bytes) => tracing::warn!(
            path = %path.display(),
            label,
            got = bytes.len(),
            expected = dst.len(),
            "backing file size mismatch, using default"
        ),
        Err(_) => tracing::info!(path = %path.display(), label, "backing file not found, using default"),
    }
}

pub fn persist_backing(path: &std::path::Path, data: &[u8], label: &str) {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        let _ = std::fs::create_dir_all(parent);
    }

    if let Err(error) = std::fs::write(path, data) {
        tracing::warn!(path = %path.display(), label, %error, "failed to persist backing file");
    }
}
