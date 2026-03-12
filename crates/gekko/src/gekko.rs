use crate::{
    cpu::{self, Cpu, semantics::Instruction},
    exi::Exi,
    flipper::{
        dsp::Dsp,
        pi::{InterruptFlag, Pi},
        vi::Vi,
    },
    mmio::Mmio,
    scheduler::{CYCLES_PER_VSYNC, EventKind, Scheduler},
};
use image::Executable;

pub struct Gekko {
    pub vsync_pending: bool,
    pub cpu: Cpu,
    pub scheduler: Scheduler,
    pub mmio: Mmio,
    pub vi: Vi,
    pub pi: Pi,
    pub dsp: Dsp,
    pub exi: Exi,
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
            vsync_pending: false,
            cpu: Cpu::new(exe.entry_point()),
            scheduler: Scheduler::new(),
            mmio,
            vi: Vi::new(),
            pi: Pi::new(),
            dsp: Dsp::new(),
            exi: Exi::dummy(),
        }
    }

    pub fn step(&mut self) {
        // Fire any events that are due
        while let Some(event) = self.scheduler.poll() {
            match event {
                EventKind::VSync => {
                    self.vsync_pending = true;
                    self.pi.assert_interrupt(InterruptFlag::Vi);
                    let next = self.scheduler.cycles + CYCLES_PER_VSYNC;
                    self.scheduler.schedule_at(next, EventKind::VSync);
                }
            }
        }

        // Deliver external interrupt when EE=1 and any enabled PI interrupt is pending
        if self.cpu.msr.external_interrupt_enable() && self.pi.interrupt_pending() {
            self.cause_external_interrupt();
            self.scheduler.cycles += 1;
            return;
        }

        // Fetch and execute next instruction
        self.cpu.cia = self.cpu.pc;
        self.cpu.nia = self.cpu.cia.wrapping_add(4);
        let instr = Instruction(self.mmio.virt_read_u32(self.cpu.cia));
        cpu::lut::dispatch(self, instr);
        self.scheduler.cycles += 1;

        self.cpu.pc = self.cpu.nia;
    }

    pub fn run_until_vsync(&mut self) {
        self.vsync_pending = false;
        while !self.vsync_pending {
            self.step();
        }
    }

    pub fn frame_size(&self) -> (usize, usize) {
        let fmt = self.vi.dcr.video_format();
        (fmt.columns(), fmt.lines())
    }
}
