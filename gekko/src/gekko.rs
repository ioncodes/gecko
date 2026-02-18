use disasm::gekko::GekkoInstruction;

use crate::{cpu, mmu, scheduler};

pub struct Gekko {
    pub cpu: cpu::Cpu,
    pub scheduler: scheduler::Scheduler,
    pub mmu: mmu::Mmu,
}

impl Gekko {
    pub fn new(path: &str) -> Self {
        let mut mmu = mmu::Mmu::new();
        let data = std::fs::read(path).expect("failed to read ROM");
        mmu.memory[..data.len()].copy_from_slice(&data);

        Gekko {
            cpu: cpu::Cpu::new(),
            scheduler: scheduler::Scheduler { cycles: 0 },
            mmu,
        }
    }

    pub fn execute_instruction(&mut self) -> Option<(GekkoInstruction, usize)> {
        let lol = disasm::gekko::GekkoInstruction::decode(&self.mmu.memory[self.cpu.pc as usize..]);
        if let Some((instr, _)) = lol {
            self.dispatch_instruction(instr);
        }

        self.cpu.pc += 4;
        self.scheduler.cycles += 1;

        lol
    }

    pub fn dispatch_instruction(&mut self, instr: disasm::gekko::GekkoInstruction) {
        match instr {
            disasm::gekko::GekkoInstruction::Bx { li, aa: false, lk: false } => {
                cpu::interpreter::branch::<false, false>(li, self);
            }
            disasm::gekko::GekkoInstruction::Bx { li, aa: true, lk: false } => {
                cpu::interpreter::branch::<false, true>(li, self);
            }
            disasm::gekko::GekkoInstruction::Bx { li, aa: false, lk: true } => {
                cpu::interpreter::branch::<true, false>(li, self);
            }
            disasm::gekko::GekkoInstruction::Bx { li, aa: true, lk: true } => {
                cpu::interpreter::branch::<true, true>(li, self);
            }
            _ => unimplemented!("Instruction not implemented: {:?}", instr),
        }
    }
}