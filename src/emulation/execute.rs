use super::{GameBoy, Instruction, MemoryBus, cpu::InterruptMasterEnable, interrupts::Interrupt};

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
        let instruction = if let Some(interrupt) = self.check_for_interrupt() {
            self.cpu.interrupt_master_enable = InterruptMasterEnable::Disabled;
            self.memory_bus.interrupt_registers_mut().clear(interrupt);

            // pandocs specify interrupts take 5 cycles to execute, but happen after
            // the next (unexecuted) opcode has been fetched. I _think_ this means
            // it'll take 6 cycles total, aligning nicely with the call instruction.
            interrupt.call_instruction()
        } else {
            let mut pc_iterator = ProgramCounterIterator {
                pc: &mut self.cpu.program_counter,
                memory_bus: &self.memory_bus,
            };
            Instruction::decode(&mut pc_iterator).unwrap()
        };

        self.cpu.execute(instruction, &mut self.memory_bus);
    }

    fn check_for_interrupt(&mut self) -> Option<Interrupt> {
        match self.cpu.interrupt_master_enable {
            InterruptMasterEnable::EnableAfterNextInstruction => {
                self.cpu.interrupt_master_enable = InterruptMasterEnable::Enabled;
                None
            }
            InterruptMasterEnable::Enabled => self.memory_bus.interrupt_registers().triggered(),
            InterruptMasterEnable::Disabled => None,
        }
    }
}
