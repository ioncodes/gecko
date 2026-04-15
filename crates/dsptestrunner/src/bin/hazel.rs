use std::sync::{Arc, Mutex};

use gecko::gamecube::GameCube;
use gecko::hooks::{AddressFilter, BusAddressFilter, HookFilters, HookFlags, HookState, Host};
use image::Dol;

const DSP_IROM: &[u8] = include_bytes!("../../../../private/dsp_rom.bin");

const STDOUT_ADDR: u32 = 0x0C00_7000;
const FAIL_COUNT_ADDR: u32 = 0x0C00_7004;
const PASS_COUNT_ADDR: u32 = 0x0C00_7008;
const CONFIG_ADDR: u32 = 0x0C00_700C;

const TOTAL_TESTS: u32 = 25000;

struct DspTestState {
    stdout_buf: String,
    finished: bool,
    fail_count: u32,
    pass_count: u32,
    config_param: u32,
}

struct DspTestHarness {
    state: Arc<Mutex<DspTestState>>,
    hook_state: HookState,
}

impl DspTestHarness {
    fn new(state: Arc<Mutex<DspTestState>>) -> Self {
        let mut flags = HookFlags::empty();
        flags |= HookFlags::BUS_WRITE_POST;
        flags |= HookFlags::BUS_READ_PRE;

        let hook_state = HookState {
            flags,
            filters: HookFilters {
                bus_write_post: BusAddressFilter {
                    phys: AddressFilter::from_addresses([STDOUT_ADDR, FAIL_COUNT_ADDR, PASS_COUNT_ADDR]),
                    ..Default::default()
                },
                bus_read_pre: BusAddressFilter {
                    phys: AddressFilter::Exact(CONFIG_ADDR),
                    ..Default::default()
                },
                ..Default::default()
            },
        };

        Self { state, hook_state }
    }
}

#[rustfmt::skip]
impl Host for DspTestHarness {
    fn hook_state(&self) -> HookState { self.hook_state.clone() }

    fn on_cpu_pre(&mut self, _emu: &mut GameCube) {}
    fn on_cpu_post(&mut self, _emu: &mut GameCube) {}

    fn on_bus_read_pre(&mut self, _emu: &mut GameCube, _virt_addr: u32, phys_addr: u32, _size: u8) -> Option<u32> {
        if phys_addr == CONFIG_ADDR {
            Some(self.state.lock().unwrap().config_param)
        } else {
            None
        }
    }

    fn on_bus_read_post(&mut self, _emu: &mut GameCube, _virt_addr: u32, _phys_addr: u32, _size: u8, value: u32) -> u32 { value }
    fn on_bus_write_pre(&mut self, _emu: &mut GameCube, _virt_addr: u32, _phys_addr: u32, _size: u8, value: u32) -> u32 { value }

    fn on_bus_write_post(&mut self, _emu: &mut GameCube, _virt_addr: u32, phys_addr: u32, _size: u8, value: u32) {
        let mut state = self.state.lock().unwrap();
        match phys_addr {
            STDOUT_ADDR => {
                let ch = (value & 0xFF) as u8;
                let done = value & 0x100 != 0;
                if ch == b'\n' {
                    println!("{}", state.stdout_buf);
                    state.stdout_buf.clear();
                } else if ch != 0 {
                    state.stdout_buf.push(ch as char);
                }
                if done && !state.stdout_buf.is_empty() {
                    println!("{}", state.stdout_buf);
                    state.stdout_buf.clear();
                }
            }
            FAIL_COUNT_ADDR => {
                state.fail_count = value;
                if value > 0 || state.fail_count + state.pass_count >= TOTAL_TESTS {
                    state.finished = true;
                }
            }
            PASS_COUNT_ADDR => {
                state.pass_count = value;
                if state.pass_count + state.fail_count >= TOTAL_TESTS {
                    state.finished = true;
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("Usage: hazel <path-to-dol>");

    let dol = Dol::parse(std::fs::read(&path).expect("Failed to read DOL file"));
    let mut gamecube = GameCube::with_image(&dol);
    gamecube.dsp.load_irom(DSP_IROM);

    let state = Arc::new(Mutex::new(DspTestState {
        stdout_buf: String::new(),
        finished: false,
        fail_count: 0,
        pass_count: 0,
        config_param: 0,
    }));
    let harness = DspTestHarness::new(state.clone());
    gamecube.set_hook_host(Box::new(harness));

    while !state.lock().unwrap().finished {
        gamecube.run_until_vsync();
    }

    let results = state.lock().unwrap();
    println!("Passed: {}, Failed: {}", results.pass_count, results.fail_count);
}
