pub mod cpu;
pub mod scheduler;
pub mod gekko;
pub mod mmio;
pub mod vi;

/// Generate `read_raw` and `write_raw` dispatch methods for a list of MMIO register types
#[macro_export]
macro_rules! impl_mmio_dispatch {
    ($($reg:ty),* $(,)?) => {
        #[inline]
        fn read_raw(&self, addr: u32, access_size: u32) -> Option<u32> {
            $(if <$reg>::fits(addr, access_size) {
                return Some(<$reg>::read_at(self, addr, access_size));
            })*
            None
        }

        #[inline]
        fn write_raw(&mut self, addr: u32, access_size: u32, val: u32) -> bool {
            $(if <$reg>::fits(addr, access_size) {
                <$reg>::write_at(self, addr, access_size, val);
                return true;
            })*
            false
        }
    };
}