pub struct Mmu {
    pub memory: Vec<u8>,
}

impl Mmu {
    pub fn new() -> Self {
        Mmu {
            memory: vec![0; 16 * 1024 * 1024],
        }
    }

    pub fn read_u8(&self, addr: u32) -> u8 {
        self.memory[addr as usize]
    }

    pub fn write_u8(&mut self, addr: u32, value: u8) {
        self.memory[addr as usize] = value;
    }
}