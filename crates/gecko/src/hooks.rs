use crate::gamecube::GameCube;

#[derive(Clone, Copy, Default)]
pub struct HookFlags(u16);

impl HookFlags {
    pub const CPU_PRE: Self = Self(1 << 0);
    pub const CPU_POST: Self = Self(1 << 1);
    pub const BUS_READ_PRE: Self = Self(1 << 2);
    pub const BUS_READ_POST: Self = Self(1 << 3);
    pub const BUS_WRITE_PRE: Self = Self(1 << 4);
    pub const BUS_WRITE_POST: Self = Self(1 << 5);

    #[inline(always)]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[inline(always)]
    pub const fn contains(&self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOrAssign for HookFlags {
    #[inline(always)]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Clone, Default)]
pub enum AddressFilter {
    #[default]
    None,
    Exact(u32),
    Set(Box<[u32]>),
}

impl AddressFilter {
    pub fn from_addresses<I>(addresses: I) -> Self
    where
        I: IntoIterator<Item = u32>,
    {
        let mut values: Vec<u32> = addresses.into_iter().collect();
        values.sort_unstable();
        values.dedup();

        match values.len() {
            0 => Self::None,
            1 => Self::Exact(values[0]),
            _ => Self::Set(values.into_boxed_slice()),
        }
    }

    #[inline(always)]
    pub fn matches(&self, address: u32) -> bool {
        match self {
            Self::None => false,
            Self::Exact(value) => *value == address,
            Self::Set(values) => values.binary_search(&address).is_ok(),
        }
    }
}

#[derive(Clone, Default)]
pub struct BusAddressFilter {
    pub virt: AddressFilter,
    pub phys: AddressFilter,
}

impl BusAddressFilter {
    #[inline(always)]
    pub fn matches(&self, virt_addr: u32, phys_addr: u32) -> bool {
        self.virt.matches(virt_addr) || self.phys.matches(phys_addr)
    }
}

#[derive(Clone, Default)]
pub struct HookFilters {
    pub cpu_pre: AddressFilter,
    pub cpu_post: AddressFilter,
    pub bus_read_pre: BusAddressFilter,
    pub bus_read_post: BusAddressFilter,
    pub bus_write_pre: BusAddressFilter,
    pub bus_write_post: BusAddressFilter,
}

#[derive(Clone, Default)]
pub struct HookState {
    pub flags: HookFlags,
    pub filters: HookFilters,
}

pub trait Host {
    /// Current cached hook state for the host.
    fn hook_state(&self) -> HookState;

    /// Forcefully rebuild trap state immediately.
    #[cfg(feature = "hooks-mut-traps")]
    fn force_refresh_traps(&mut self) -> Result<HookState, String> {
        Ok(self.hook_state())
    }

    /// Apply any pending trap refresh requested by the script.
    #[cfg(feature = "hooks-mut-traps")]
    fn take_pending_hook_state(&mut self) -> Result<Option<HookState>, String> {
        Ok(None)
    }

    /// Drain buffered log messages emitted by the script.
    fn drain_logs(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Called before each CPU instruction executes.
    fn on_cpu_pre(&mut self, emu: &mut GameCube);

    /// Called after each CPU instruction executes.
    fn on_cpu_post(&mut self, emu: &mut GameCube);

    /// Called before a bus read. Return Some(val) to override the read value.
    fn on_bus_read_pre(&mut self, emu: &mut GameCube, virt_addr: u32, phys_addr: u32, size: u8) -> Option<u32>;

    /// Called after a bus read completes. Returns the (possibly modified) value.
    fn on_bus_read_post(&mut self, emu: &mut GameCube, virt_addr: u32, phys_addr: u32, size: u8, value: u32) -> u32;

    /// Called before a bus write. Returns the (possibly modified) value to write.
    fn on_bus_write_pre(&mut self, emu: &mut GameCube, virt_addr: u32, phys_addr: u32, size: u8, value: u32) -> u32;

    /// Called after a bus write completes.
    fn on_bus_write_post(&mut self, emu: &mut GameCube, virt_addr: u32, phys_addr: u32, size: u8, value: u32);
}
