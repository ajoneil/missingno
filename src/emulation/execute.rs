use super::{GameBoy, Instruction, MemoryBus};

struct ProgramCounterIterator<'a> {
    pc: &'a mut u16,
    memory_bus: &'a MemoryBus,
}

impl<'a> Iterator for ProgramCounterIterator<'a> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.memory_bus.read(*self.pc);
        *self.pc += 1;
        Some(value)
    }
}

impl GameBoy {
    pub fn step(&mut self) {
        let mut pc_iterator = ProgramCounterIterator {
            pc: &mut self.cpu.program_counter,
            memory_bus: &self.memory_bus,
        };
        let instruction = Instruction::decode(&mut pc_iterator).unwrap();
        let memory_bus = &mut self.memory_bus;

        self.cpu.execute(instruction, memory_bus);
    }
}
