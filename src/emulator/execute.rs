use super::{
    GameBoy,
    cpu::{InterruptMasterEnable, execute::OpResult, instructions::Instruction},
    interrupts::Interrupt,
};

impl Iterator for GameBoy {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.mapped.read(self.cpu.program_counter);
        self.cpu.program_counter += 1;
        Some(value)
    }
}

impl GameBoy {
    pub fn step(&mut self) {
        let pc = self.cpu.program_counter;
        let instruction = if let Some(interrupt) = self.check_for_interrupt() {
            self.cpu.interrupt_master_enable = InterruptMasterEnable::Disabled;
            self.mapped.interrupts.clear(interrupt);

            // pandocs specify interrupts take 5 cycles to execute, but happen after
            // the next (unexecuted) opcode has been fetched. I _think_ this means
            // it'll take 6 cycles total, aligning nicely with the call instruction.
            interrupt.call_instruction()
        } else {
            Instruction::decode(self).unwrap()
        };

        dbg!(format!("{:04x}: {}", pc, instruction));

        let OpResult(cycles, memory_write) = self.cpu.execute(instruction, &self.mapped);
        if let Some(memory_write) = memory_write {
            self.mapped.write(memory_write);
        }

        self.mapped.video.step(cycles);

        // TODO: Cycles/timers
    }

    fn check_for_interrupt(&mut self) -> Option<Interrupt> {
        match self.cpu.interrupt_master_enable {
            InterruptMasterEnable::EnableAfterNextInstruction => {
                self.cpu.interrupt_master_enable = InterruptMasterEnable::Enabled;
                None
            }
            InterruptMasterEnable::Enabled => self.mapped.interrupts.triggered(),
            InterruptMasterEnable::Disabled => None,
        }
    }
}
