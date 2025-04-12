use crate::emulation::{
    Cpu,
    cpu::{InterruptMasterEnable, cycles::Cycles, instructions::Interrupt},
};

impl Cpu {
    pub fn execute_interrupt(&mut self, instruction: Interrupt) -> Cycles {
        match instruction {
            Interrupt::Enable => {
                self.interrupt_master_enable = InterruptMasterEnable::EnableAfterNextInstruction;
                Cycles(1)
            }
            Interrupt::Disable => {
                self.interrupt_master_enable = InterruptMasterEnable::Disabled;
                Cycles(1)
            }
            Interrupt::Await => todo!(),
        }
    }
}
