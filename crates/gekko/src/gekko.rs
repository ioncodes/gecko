use crate::{
    cpu::{self, semantics::Instruction},
    mmu, scheduler,
};

pub struct Gekko {
    pub cpu: cpu::Cpu,
    pub scheduler: scheduler::Scheduler,
    pub mmu: mmu::Mmu,
}

impl Gekko {
    pub fn new(path: &str) -> Self {
        let mut mmu = mmu::Mmu::new();
        let data = std::fs::read(path).expect("failed to read ROM");
        let dol = dol::Dol::parse(&data);

        // Copy TEXT sections to memory
        for section in &dol.text_sections {
            for i in 0..section.size {
                let addr = section.vaddr + i;
                let value = data[(section.offset + i) as usize];
                mmu.virt_write_u8(addr, value);
            }
        }

        // Copy DATA sections to memory
        for section in &dol.data_sections {
            for i in 0..section.size {
                let addr = section.vaddr + i;
                let value = data[(section.offset + i) as usize];
                mmu.virt_write_u8(addr, value);
            }
        }

        // Zero out the BSS section
        for i in 0..dol.bss_size {
            let addr = dol.bss_start + i;
            mmu.virt_write_u8(addr, 0);
        }

        Gekko {
            cpu: cpu::Cpu::new(dol.entry_point),
            scheduler: scheduler::Scheduler { cycles: 0 },
            mmu,
        }
    }

    pub fn run_until_event(&mut self) {
        self.cpu.cia = self.cpu.pc;
        self.cpu.nia = self.cpu.cia.wrapping_add(4);

        let instr = Instruction(self.mmu.virt_read_u32(self.cpu.cia));
        cpu::lut::dispatch(self, instr);
        self.scheduler.cycles += 1;

        self.cpu.pc = self.cpu.nia;
    }
}
