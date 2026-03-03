use crate::{
    cpu::{self, Cpu, semantics::Instruction},
    mmio::Mmio,
    scheduler::Scheduler,
    vi::Vi,
};
use image::Executable;

pub struct Gekko {
    pub cpu: Cpu,
    pub scheduler: Scheduler,
    pub mmio: Mmio,
    pub vi: Vi,
}

impl Gekko {
    pub fn new(exe: &impl Executable) -> Self {
        let mut mmio = Mmio::new();
        let data = exe.data();

        // Copy TEXT sections to memory
        for section in exe.text_sections() {
            for i in 0..section.size {
                let addr = section.vaddr + i;
                let value = data[(section.offset + i) as usize];
                mmio.virt_write_u8(addr, value);
            }
        }

        // Copy DATA sections to memory
        for section in exe.data_sections() {
            for i in 0..section.size {
                let addr = section.vaddr + i;
                let value = data[(section.offset + i) as usize];
                mmio.virt_write_u8(addr, value);
            }
        }

        // Zero out the BSS section
        let (bss_start, bss_size) = exe.bss();
        for i in 0..bss_size {
            let addr = bss_start + i;
            mmio.virt_write_u8(addr, 0);
        }

        Gekko {
            cpu: Cpu::new(exe.entry_point()),
            scheduler: Scheduler { cycles: 0 },
            mmio,
            vi: Vi::new(),
        }
    }

    pub fn run_until_event(&mut self) {
        self.cpu.cia = self.cpu.pc;
        self.cpu.nia = self.cpu.cia.wrapping_add(4);

        // Bypass the bus, instructions are always fetched from RAM
        let instr = Instruction(self.mmio.virt_read_u32(self.cpu.cia));
        cpu::lut::dispatch(self, instr);
        self.scheduler.cycles += 1;

        self.cpu.pc = self.cpu.nia;
    }
}
