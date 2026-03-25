use std::collections::HashMap;
#[cfg(feature = "scripting-mut-traps")]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use gekko::gekko::Gekko;
use gekko::scripting::{AddressFilter, BusAddressFilter, HookFlags, ScriptHookFilters, ScriptHookState, ScriptHost};
use mlua::{Function, Lua, RegistryKey, Result as LuaResult, Table, UserDataMethods, Value};

pub struct LuaScriptHost {
    lua: Lua,
    flags: HookFlags,
    #[cfg(feature = "scripting-mut-traps")]
    refresh_requested: Arc<AtomicBool>,
    cpu_pre: CpuHookDispatch,
    cpu_post: CpuHookDispatch,
    bus_read_pre: BusHookDispatch,
    bus_read_post: BusHookDispatch,
    bus_write_pre: BusHookDispatch,
    bus_write_post: BusHookDispatch,
}

type CpuHookDispatch = HashMap<u32, LuaCallback>;

struct LoadedDispatches {
    state: ScriptHookState,
    cpu_pre: CpuHookDispatch,
    cpu_post: CpuHookDispatch,
    bus_read_pre: BusHookDispatch,
    bus_read_post: BusHookDispatch,
    bus_write_pre: BusHookDispatch,
    bus_write_post: BusHookDispatch,
}

#[derive(Default)]
struct BusHookDispatch {
    virt_traps: HashMap<u32, LuaCallback>,
    phys_traps: HashMap<u32, LuaCallback>,
}

impl BusHookDispatch {
    fn has_handler(&self) -> bool {
        !self.virt_traps.is_empty() || !self.phys_traps.is_empty()
    }

    fn resolve(&self, virt_addr: u32, phys_addr: u32) -> Option<&LuaCallback> {
        self.virt_traps
            .get(&virt_addr)
            .or_else(|| self.phys_traps.get(&phys_addr))
    }

    fn filter(&self) -> BusAddressFilter {
        BusAddressFilter {
            virt: AddressFilter::from_addresses(self.virt_traps.keys().copied()),
            phys: AddressFilter::from_addresses(self.phys_traps.keys().copied()),
        }
    }
}

impl LoadedDispatches {
    fn from_lua(lua: &Lua) -> LuaResult<Self> {
        let globals = lua.globals();
        let trap_root = load_optional_table(&globals, "traps", "traps")?;
        let cpu_pre_traps = match trap_root.as_ref() {
            Some(root) => load_optional_table(root, "cpu_pre", "traps.cpu_pre")?,
            None => None,
        };
        let cpu_post_traps = match trap_root.as_ref() {
            Some(root) => load_optional_table(root, "cpu_post", "traps.cpu_post")?,
            None => None,
        };

        let cpu_pre = load_address_traps(lua, cpu_pre_traps.as_ref(), "traps.cpu_pre")?;
        let cpu_post = load_address_traps(lua, cpu_post_traps.as_ref(), "traps.cpu_post")?;
        let bus_read_pre = load_bus_dispatch(lua, trap_root.as_ref(), "bus_read_pre")?;
        let bus_read_post = load_bus_dispatch(lua, trap_root.as_ref(), "bus_read_post")?;
        let bus_write_pre = load_bus_dispatch(lua, trap_root.as_ref(), "bus_write_pre")?;
        let bus_write_post = load_bus_dispatch(lua, trap_root.as_ref(), "bus_write_post")?;

        let mut flags = HookFlags::empty();
        if !cpu_pre.is_empty() {
            flags |= HookFlags::CPU_PRE;
        }
        if !cpu_post.is_empty() {
            flags |= HookFlags::CPU_POST;
        }
        if bus_read_pre.has_handler() {
            flags |= HookFlags::BUS_READ_PRE;
        }
        if bus_read_post.has_handler() {
            flags |= HookFlags::BUS_READ_POST;
        }
        if bus_write_pre.has_handler() {
            flags |= HookFlags::BUS_WRITE_PRE;
        }
        if bus_write_post.has_handler() {
            flags |= HookFlags::BUS_WRITE_POST;
        }

        Ok(Self {
            state: ScriptHookState {
                flags,
                filters: ScriptHookFilters {
                    cpu_pre: AddressFilter::from_addresses(cpu_pre.keys().copied()),
                    cpu_post: AddressFilter::from_addresses(cpu_post.keys().copied()),
                    bus_read_pre: bus_read_pre.filter(),
                    bus_read_post: bus_read_post.filter(),
                    bus_write_pre: bus_write_pre.filter(),
                    bus_write_post: bus_write_post.filter(),
                },
            },
            cpu_pre,
            cpu_post,
            bus_read_pre,
            bus_read_post,
            bus_write_pre,
            bus_write_post,
        })
    }
}

enum LuaCallback {
    Stored(RegistryKey),
}

impl LuaCallback {
    fn from_function(lua: &Lua, function: Function) -> LuaResult<Self> {
        Ok(Self::Stored(lua.create_registry_value(function)?))
    }

    fn from_value(lua: &Lua, value: Value, context: &str) -> LuaResult<Self> {
        match value {
            Value::Function(function) => Self::from_function(lua, function),
            other => Err(mlua::Error::runtime(format!(
                "{context} must map to a Lua function, got {}",
                other.type_name()
            ))),
        }
    }

    fn resolve(&self, lua: &Lua) -> LuaResult<Function> {
        match self {
            Self::Stored(key) => lua.registry_value(key),
        }
    }
}

impl LuaScriptHost {
    pub fn from_file(path: &str) -> LuaResult<Self> {
        let source = std::fs::read_to_string(path).map_err(|e| mlua::Error::runtime(e.to_string()))?;
        Self::from_source(path, &source)
    }

    pub fn from_source(name: &str, source: &str) -> LuaResult<Self> {
        let lua = Lua::new();
        #[cfg(feature = "scripting-mut-traps")]
        let refresh_requested = Arc::new(AtomicBool::new(false));

        let log_fn = lua.create_function(|_, msg: String| {
            tracing::info!("{}", msg);
            Ok(())
        })?;
        lua.globals().set("log", log_fn)?;

        #[cfg(feature = "scripting-mut-traps")]
        {
            let refresh_requested_fn = refresh_requested.clone();
            let refresh_fn = lua.create_function(move |_, ()| {
                refresh_requested_fn.store(true, Ordering::Relaxed);
                Ok(())
            })?;
            lua.globals().set("refresh_traps", refresh_fn)?;
        }

        lua.load(source).set_name(name).exec()?;

        let mut host = LuaScriptHost {
            lua,
            flags: HookFlags::empty(),
            #[cfg(feature = "scripting-mut-traps")]
            refresh_requested,
            cpu_pre: CpuHookDispatch::default(),
            cpu_post: CpuHookDispatch::default(),
            bus_read_pre: BusHookDispatch::default(),
            bus_read_post: BusHookDispatch::default(),
            bus_write_pre: BusHookDispatch::default(),
            bus_write_post: BusHookDispatch::default(),
        };
        host.reload_dispatches()?;
        Ok(host)
    }

    fn reload_dispatches(&mut self) -> LuaResult<ScriptHookState> {
        let loaded = LoadedDispatches::from_lua(&self.lua)?;
        let state = loaded.state.clone();
        self.flags = state.flags;
        self.cpu_pre = loaded.cpu_pre;
        self.cpu_post = loaded.cpu_post;
        self.bus_read_pre = loaded.bus_read_pre;
        self.bus_read_post = loaded.bus_read_post;
        self.bus_write_pre = loaded.bus_write_pre;
        self.bus_write_post = loaded.bus_write_post;
        #[cfg(feature = "scripting-mut-traps")]
        self.refresh_requested.store(false, Ordering::Relaxed);
        Ok(state)
    }

    fn register_emu_methods(methods: &mut impl UserDataMethods<GekkoRef>) {
        methods.add_method("gpr", |_, this, i: u8| Ok(this.cpu.read_gpr(i)));
        methods.add_method_mut("set_gpr", |_, this, (i, val): (u8, u32)| {
            this.cpu.write_gpr(i, val);
            Ok(())
        });
        methods.add_method("fpr", |_, this, i: u8| Ok(this.cpu.read_fpr(i)));
        methods.add_method_mut("set_fpr", |_, this, (i, val): (u8, f64)| {
            this.cpu.write_fpr(i, val);
            Ok(())
        });
        methods.add_method("ps1", |_, this, i: u8| Ok(this.cpu.read_ps1(i)));
        methods.add_method_mut("set_ps1", |_, this, (i, val): (u8, f64)| {
            this.cpu.write_ps1(i, val);
            Ok(())
        });
        methods.add_method("pc", |_, this, ()| Ok(this.cpu.pc));
        methods.add_method_mut("set_pc", |_, this, val: u32| {
            this.cpu.nia = val;
            Ok(())
        });
        methods.add_method("lr", |_, this, ()| Ok(this.cpu.spr.lr));
        methods.add_method_mut("set_lr", |_, this, val: u32| {
            this.cpu.spr.lr = val;
            Ok(())
        });
        methods.add_method("ctr", |_, this, ()| Ok(this.cpu.spr.ctr));
        methods.add_method_mut("set_ctr", |_, this, val: u32| {
            this.cpu.spr.ctr = val;
            Ok(())
        });
        methods.add_method("cr", |_, this, ()| Ok(this.cpu.cr.raw()));
        methods.add_method("msr", |_, this, ()| Ok(this.cpu.msr.raw()));
        methods.add_method("xer", |_, this, ()| Ok(this.cpu.spr.xer.raw()));
        methods.add_method("fpscr", |_, this, ()| Ok(this.cpu.fpscr));
        methods.add_method("cycles", |_, this, ()| Ok(this.scheduler.cycles));

        methods.add_method("read_u8", |_, this, addr: u32| Ok(this.mmio.virt_read_u8(addr)));
        methods.add_method("read_u16", |_, this, addr: u32| Ok(this.mmio.virt_read_u16(addr)));
        methods.add_method("read_u32", |_, this, addr: u32| Ok(this.mmio.virt_read_u32(addr)));
        methods.add_method_mut("write_u8", |_, this, (addr, val): (u32, u8)| {
            this.mmio.virt_write_u8(addr, val);
            Ok(())
        });
        methods.add_method_mut("write_u16", |_, this, (addr, val): (u32, u16)| {
            this.mmio.virt_write_u16(addr, val);
            Ok(())
        });
        methods.add_method_mut("write_u32", |_, this, (addr, val): (u32, u32)| {
            this.mmio.virt_write_u32(addr, val);
            Ok(())
        });
        methods.add_method("read_phys_u8", |_, this, addr: u32| Ok(this.mmio.phys_read_u8(addr)));
        methods.add_method("read_phys_u16", |_, this, addr: u32| Ok(this.mmio.phys_read_u16(addr)));
        methods.add_method("read_phys_u32", |_, this, addr: u32| Ok(this.mmio.phys_read_u32(addr)));
        methods.add_method_mut("write_phys_u8", |_, this, (addr, val): (u32, u8)| {
            this.mmio.phys_write_u8(addr, val);
            Ok(())
        });
        methods.add_method_mut("write_phys_u16", |_, this, (addr, val): (u32, u16)| {
            this.mmio.phys_write_u16(addr, val);
            Ok(())
        });
        methods.add_method_mut("write_phys_u32", |_, this, (addr, val): (u32, u32)| {
            this.mmio.phys_write_u32(addr, val);
            Ok(())
        });
        methods.add_method("virt_to_phys", |_, _this, addr: u32| {
            Ok(gekko::mmio::Mmio::virt_to_phys(addr))
        });
    }

    fn call_cpu_hook(&self, hook_name: &str, callback: &LuaCallback, emu: &mut Gekko) -> LuaResult<()> {
        self.lua
            .scope(|scope| {
                let ud = scope.create_userdata(GekkoRef(emu as *mut Gekko))?;
                let func = callback.resolve(&self.lua)?;
                func.call::<()>(&ud)
            })
            .map_err(|err| annotate_hook_error(hook_name, err))
    }

    fn call_bus_read_pre_hook(
        &self,
        hook_name: &str,
        callback: &LuaCallback,
        emu: &mut Gekko,
        virt_addr: u32,
        phys_addr: u32,
        size: u8,
    ) -> LuaResult<Option<u32>> {
        self.lua
            .scope(|scope| {
                let ud = scope.create_userdata(GekkoRef(emu as *mut Gekko))?;
                let func = callback.resolve(&self.lua)?;
                func.call::<Option<u32>>((&ud, virt_addr, phys_addr, size))
            })
            .map_err(|err| annotate_hook_error(hook_name, err))
    }

    fn call_bus_read_post_hook(
        &self,
        hook_name: &str,
        callback: &LuaCallback,
        emu: &mut Gekko,
        virt_addr: u32,
        phys_addr: u32,
        size: u8,
        value: u32,
    ) -> LuaResult<()> {
        self.lua
            .scope(|scope| {
                let ud = scope.create_userdata(GekkoRef(emu as *mut Gekko))?;
                let func = callback.resolve(&self.lua)?;
                func.call::<()>((&ud, virt_addr, phys_addr, size, value))
            })
            .map_err(|err| annotate_hook_error(hook_name, err))
    }

    fn call_bus_write_pre_hook(
        &self,
        hook_name: &str,
        callback: &LuaCallback,
        emu: &mut Gekko,
        virt_addr: u32,
        phys_addr: u32,
        size: u8,
        value: u32,
    ) -> LuaResult<Option<u32>> {
        self.lua
            .scope(|scope| {
                let ud = scope.create_userdata(GekkoRef(emu as *mut Gekko))?;
                let func = callback.resolve(&self.lua)?;
                func.call::<Option<u32>>((&ud, virt_addr, phys_addr, size, value))
            })
            .map_err(|err| annotate_hook_error(hook_name, err))
    }

    fn call_bus_write_post_hook(
        &self,
        hook_name: &str,
        callback: &LuaCallback,
        emu: &mut Gekko,
        virt_addr: u32,
        phys_addr: u32,
        size: u8,
        value: u32,
    ) -> LuaResult<()> {
        self.lua
            .scope(|scope| {
                let ud = scope.create_userdata(GekkoRef(emu as *mut Gekko))?;
                let func = callback.resolve(&self.lua)?;
                func.call::<()>((&ud, virt_addr, phys_addr, size, value))
            })
            .map_err(|err| annotate_hook_error(hook_name, err))
    }
}

fn annotate_hook_error(hook_name: &str, err: mlua::Error) -> mlua::Error {
    mlua::Error::runtime(format!("{hook_name} failed: {err}"))
}

fn log_hook_error(hook_name: &str, err: &mlua::Error) {
    tracing::error!(hook = hook_name, error = %err, "script hook failed");
}

fn load_optional_table(parent: &Table, key: &str, context: &str) -> LuaResult<Option<Table>> {
    match parent.get::<Value>(key)? {
        Value::Nil => Ok(None),
        Value::Table(table) => Ok(Some(table)),
        other => Err(mlua::Error::runtime(format!(
            "{context} must be a table, got {}",
            other.type_name()
        ))),
    }
}

fn load_address_traps(lua: &Lua, table: Option<&Table>, context: &str) -> LuaResult<HashMap<u32, LuaCallback>> {
    let Some(table) = table else {
        return Ok(HashMap::new());
    };

    let mut traps = HashMap::new();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let address = parse_address_key(key, context)?;
        let callback = LuaCallback::from_value(lua, value, context)?;
        traps.insert(address, callback);
    }
    Ok(traps)
}

fn load_bus_dispatch(lua: &Lua, trap_root: Option<&Table>, trap_name: &str) -> LuaResult<BusHookDispatch> {
    let trap_table = trap_root
        .map(|root| load_optional_table(root, trap_name, &format!("traps.{trap_name}")))
        .transpose()?
        .flatten();

    let mut dispatch = BusHookDispatch::default();

    let Some(table) = trap_table else {
        return Ok(dispatch);
    };

    let virt_table = load_optional_table(&table, "virt", &format!("traps.{trap_name}.virt"))?;
    let phys_table = load_optional_table(&table, "phys", &format!("traps.{trap_name}.phys"))?;

    if virt_table.is_some() || phys_table.is_some() {
        dispatch.virt_traps = load_address_traps(lua, virt_table.as_ref(), &format!("traps.{trap_name}.virt"))?;
        dispatch.phys_traps = load_address_traps(lua, phys_table.as_ref(), &format!("traps.{trap_name}.phys"))?;
    } else {
        dispatch.virt_traps = load_address_traps(lua, Some(&table), &format!("traps.{trap_name}"))?;
    }

    Ok(dispatch)
}

fn parse_address_key(value: Value, context: &str) -> LuaResult<u32> {
    match value {
        Value::Integer(address) if (0..=u32::MAX as i64).contains(&address) => Ok(address as u32),
        Value::Number(address)
            if address.is_finite() && address.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&address) =>
        {
            Ok(address as u32)
        }
        other => Err(mlua::Error::runtime(format!(
            "{context} keys must be integer addresses, got {}",
            other.type_name()
        ))),
    }
}

struct GekkoRef(*mut Gekko);

unsafe impl Send for GekkoRef {}

impl mlua::UserData for GekkoRef {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        LuaScriptHost::register_emu_methods(methods);
    }
}

impl std::ops::Deref for GekkoRef {
    type Target = Gekko;

    fn deref(&self) -> &Gekko {
        unsafe { &*self.0 }
    }
}

impl std::ops::DerefMut for GekkoRef {
    fn deref_mut(&mut self) -> &mut Gekko {
        unsafe { &mut *self.0 }
    }
}

impl ScriptHost for LuaScriptHost {
    fn hook_state(&self) -> ScriptHookState {
        ScriptHookState {
            flags: self.flags,
            filters: ScriptHookFilters {
                cpu_pre: AddressFilter::from_addresses(self.cpu_pre.keys().copied()),
                cpu_post: AddressFilter::from_addresses(self.cpu_post.keys().copied()),
                bus_read_pre: self.bus_read_pre.filter(),
                bus_read_post: self.bus_read_post.filter(),
                bus_write_pre: self.bus_write_pre.filter(),
                bus_write_post: self.bus_write_post.filter(),
            },
        }
    }

    #[cfg(feature = "scripting-mut-traps")]
    fn force_refresh_traps(&mut self) -> Result<ScriptHookState, String> {
        self.reload_dispatches().map_err(|err| err.to_string())
    }

    #[cfg(feature = "scripting-mut-traps")]
    fn take_pending_hook_state(&mut self) -> Result<Option<ScriptHookState>, String> {
        if !self.refresh_requested.swap(false, Ordering::Relaxed) {
            return Ok(None);
        }

        self.reload_dispatches().map(Some).map_err(|err| err.to_string())
    }

    fn on_cpu_pre(&mut self, emu: &mut Gekko) {
        let pc = emu.cpu.pc;
        if let Some(callback) = self.cpu_pre.get(&pc) {
            if let Err(err) = self.call_cpu_hook("cpu_pre", callback, emu) {
                log_hook_error("cpu_pre", &err);
            }
        }
    }

    fn on_cpu_post(&mut self, emu: &mut Gekko) {
        let pc = emu.cpu.cia;
        if let Some(callback) = self.cpu_post.get(&pc) {
            if let Err(err) = self.call_cpu_hook("cpu_post", callback, emu) {
                log_hook_error("cpu_post", &err);
            }
        }
    }

    fn on_bus_read_pre(&mut self, emu: &mut Gekko, virt_addr: u32, phys_addr: u32, size: u8) -> Option<u32> {
        let callback = self.bus_read_pre.resolve(virt_addr, phys_addr)?;
        match self.call_bus_read_pre_hook("bus_read_pre", callback, emu, virt_addr, phys_addr, size) {
            Ok(value) => value,
            Err(err) => {
                log_hook_error("bus_read_pre", &err);
                None
            }
        }
    }

    fn on_bus_read_post(&mut self, emu: &mut Gekko, virt_addr: u32, phys_addr: u32, size: u8, value: u32) {
        if let Some(callback) = self.bus_read_post.resolve(virt_addr, phys_addr) {
            if let Err(err) =
                self.call_bus_read_post_hook("bus_read_post", callback, emu, virt_addr, phys_addr, size, value)
            {
                log_hook_error("bus_read_post", &err);
            }
        }
    }

    fn on_bus_write_pre(&mut self, emu: &mut Gekko, virt_addr: u32, phys_addr: u32, size: u8, value: u32) -> u32 {
        let Some(callback) = self.bus_write_pre.resolve(virt_addr, phys_addr) else {
            return value;
        };

        match self.call_bus_write_pre_hook("bus_write_pre", callback, emu, virt_addr, phys_addr, size, value) {
            Ok(Some(updated)) => updated,
            Ok(None) => value,
            Err(err) => {
                log_hook_error("bus_write_pre", &err);
                value
            }
        }
    }

    fn on_bus_write_post(&mut self, emu: &mut Gekko, virt_addr: u32, phys_addr: u32, size: u8, value: u32) {
        if let Some(callback) = self.bus_write_post.resolve(virt_addr, phys_addr) {
            if let Err(err) =
                self.call_bus_write_post_hook("bus_write_post", callback, emu, virt_addr, phys_addr, size, value)
            {
                log_hook_error("bus_write_post", &err);
            }
        }
    }
}
