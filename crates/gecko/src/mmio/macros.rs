#[macro_export]
macro_rules! mmio_device_dispatch {
    (
        read = $read_fn:ident,
        write = $write_fn:ident,
        registers = [ $($reg:ty),* $(,)? ] $(,)?
    ) => {
        #[inline(always)]
        pub fn $read_fn<const SYSTEM: $crate::system::SystemId>(
            sys: &mut $crate::system::System<SYSTEM>,
            addr: u32,
            size: u32,
        ) -> Option<u32> {
            $(
                if <$reg as $crate::mmio::traits::MmioRegister>::fits(addr, size) {
                    return Some(
                        <$reg as $crate::mmio::traits::MmioAccess<$crate::system::System<SYSTEM>>>
                            ::read_at(sys, addr, size),
                    );
                }
            )*
            None
        }

        #[inline(always)]
        pub fn $write_fn<const SYSTEM: $crate::system::SystemId>(
            sys: &mut $crate::system::System<SYSTEM>,
            addr: u32,
            size: u32,
            val: u32,
        ) -> bool {
            $(
                if <$reg as $crate::mmio::traits::MmioRegister>::fits(addr, size) {
                    tracing::debug!(
                        reg = stringify!($reg),
                        addr = format!("{addr:08X}"),
                        val = format!("{val:08X}"),
                        size,
                        "MMIO write"
                    );
                    <$reg as $crate::mmio::traits::MmioAccess<$crate::system::System<SYSTEM>>>
                        ::write_at(sys, addr, size, val);
                    return true;
                }
            )*
            false
        }
    };
}

#[macro_export]
macro_rules! mmio_reg {
    ($name:ident : $raw:tt @ $addr:tt) => {
        impl $crate::mmio::traits::MmioRegister for $name {
            const ADDR: u32 = $crate::mmio::Mmio::virt_to_phys($addr);
            const SIZE: usize = ::core::mem::size_of::<$raw>();
            #[inline(always)]
            fn from_raw(raw: u32) -> Self {
                (raw as $raw).into()
            }
            #[inline(always)]
            fn to_raw(self) -> u32 {
                self.raw() as u32
            }
        }
    };
}

#[macro_export]
macro_rules! mmio_default_access {
    ($reg:ty => System . $($field:ident).+) => {
        impl<const SYSTEM: $crate::system::SystemId>
            $crate::mmio::traits::MmioAccess<$crate::system::System<SYSTEM>> for $reg
        {
            #[inline(always)]
            fn read(sys: &mut $crate::system::System<SYSTEM>) -> Self {
                sys . $($field).+
            }
            #[inline(always)]
            fn write(
                self,
                sys: &mut $crate::system::System<SYSTEM>,
                _: $crate::mmio::traits::WriteMask,
            ) {
                sys . $($field).+ = self;
            }
        }
    };
}
