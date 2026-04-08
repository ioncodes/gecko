#[macro_export]
macro_rules! mmio_device_dispatch {
    (
        read = $read_fn:ident,
        write = $write_fn:ident,
        registers = [ $($reg:ty),* $(,)? ] $(,)?
    ) => {
        #[inline]
        pub fn $read_fn(
            gc: &mut $crate::gamecube::GameCube,
            addr: u32,
            size: u32,
        ) -> Option<u32> {
            $(
                if <$reg as $crate::mmio::traits::MmioRegister>::fits(addr, size) {
                    return Some(
                        <$reg as $crate::mmio::traits::MmioHandler<$crate::gamecube::GameCube>>
                            ::read_at(gc, addr, size),
                    );
                }
            )*
            None
        }

        #[inline]
        pub fn $write_fn(
            gc: &mut $crate::gamecube::GameCube,
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
                    <$reg as $crate::mmio::traits::MmioHandler<$crate::gamecube::GameCube>>
                        ::write_at(gc, addr, size, val);
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
    ($reg:ty => GameCube . $($field:ident).+) => {
        impl $crate::mmio::traits::MmioHandler<$crate::gamecube::GameCube> for $reg {
            #[inline(always)]
            fn read(gc: &mut $crate::gamecube::GameCube) -> Self {
                gc . $($field).+
            }
            #[inline(always)]
            fn write(
                self,
                gc: &mut $crate::gamecube::GameCube,
                _: $crate::mmio::traits::WriteMask,
            ) {
                gc . $($field).+ = self;
            }
        }
    };
}

/// Generate `read_raw` and `write_raw` dispatch methods for a list of MMIO register types
/// Inserts two methods into the enclosing `impl` block that iterate over the provided
/// register types and delegate reads/writes to the appropriate `MmioAccess` impl
#[macro_export]
macro_rules! impl_mmio_dispatch {
    ($($reg:ty),* $(,)?) => {
        #[inline]
        fn read_raw(&mut self, addr: u32, access_size: u32) -> Option<u32> {
            $(if <$reg>::fits(addr, access_size) {
                return Some(<$reg>::read_at(self, addr, access_size));
            })*
            None
        }

        #[inline]
        fn write_raw(&mut self, addr: u32, access_size: u32, val: u32) -> bool {
            $(if <$reg>::fits(addr, access_size) {
                tracing::debug!(
                    reg = stringify!($reg),
                    addr = format!("{addr:08X}"),
                    val = format!("{val:08X}"),
                    size = access_size,
                    "MMIO write"
                );
                <$reg>::write_at(self, addr, access_size, val);
                return true;
            })*
            false
        }
    };
}

/// Define a bitfield register struct, implement `MmioRegister`, and optionally
/// implement `MmioAccess`
///
/// ```ignore
/// mmio_register! {
///     MyReg: u16 @ 0xCC001234 => MyDevice.my_field {
///         #[bits(0..=7, alias = "val")] pub value: u8,
///     }
/// }
/// ```
///
/// or
///
/// ```ignore
/// mmio_register! {
///     MyReg: u16 @ 0xCC001234 {
///         #[bits(0..=7, alias = "val")] pub value: u8,
///     }
/// }
///
/// impl MmioAccess<MyDevice> for MyReg { ... }
/// ```
#[macro_export]
macro_rules! mmio_register {
    // Auto MmioAccess: Name: raw @ addr => Owner.field { ... }
    (
        $(#[$attr:meta])*
        $name:ident : $raw:tt @ $addr:tt => $owner:tt . $field:ident {
            $($body:tt)*
        }
    ) => {
        #[chapa::bitfield($raw, order = lsb0)]
        #[derive(Copy, Clone, Debug)]
        $(#[$attr])*
        pub struct $name {
            $($body)*
        }

        impl $crate::mmio::traits::MmioRegister for $name {
            const ADDR: u32 = $crate::mmio::Mmio::virt_to_phys($addr);
            const SIZE: usize = ::core::mem::size_of::<$raw>();
            fn from_raw(raw: u32) -> Self { (raw as $raw).into() }
            fn to_raw(self) -> u32 { self.raw() as u32 }
        }

        impl $crate::mmio::traits::MmioAccess<$owner> for $name {
            fn read(dev: &$owner) -> Self { dev.$field }
            fn write(self, dev: &mut $owner) { dev.$field = self; }
        }
    };

    // Manual MmioAccess: Name: raw @ addr { ... }
    (
        $(#[$attr:meta])*
        $name:ident : $raw:tt @ $addr:tt {
            $($body:tt)*
        }
    ) => {
        #[chapa::bitfield($raw, order = lsb0)]
        #[derive(Copy, Clone, Debug)]
        $(#[$attr])*
        pub struct $name {
            $($body)*
        }

        impl $crate::mmio::traits::MmioRegister for $name {
            const ADDR: u32 = $crate::mmio::Mmio::virt_to_phys($addr);
            const SIZE: usize = ::core::mem::size_of::<$raw>();
            fn from_raw(raw: u32) -> Self { (raw as $raw).into() }
            fn to_raw(self) -> u32 { self.raw() as u32 }
        }
    };
}
