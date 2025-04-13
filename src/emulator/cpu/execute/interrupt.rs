use super::OpResult;
use crate::emulator::{
    Cpu,
    cpu::{InterruptMasterEnable, instructions::Interrupt},
};

impl Cpu {
    pub fn execute_interrupt(&mut self, instruction: Interrupt) -> OpResult {
        match instruction {
            Interrupt::Enable => {
                self.interrupt_master_enable = InterruptMasterEnable::EnableAfterNextInstruction;
                OpResult::cycles(1)
            }
            Interrupt::Disable => {
                self.interrupt_master_enable = InterruptMasterEnable::Disabled;
                OpResult::cycles(1)
            }
            Interrupt::Await => todo!(),
        }
    }
}
