use super::OpResult;
use crate::game_boy::{
    Cpu,
    cpu::{InterruptMasterEnable, instructions::Interrupt},
};

impl Cpu {
    pub fn execute_interrupt(&mut self, instruction: Interrupt) -> OpResult {
        match instruction {
            Interrupt::Enable => {
                if self.interrupt_master_enable != InterruptMasterEnable::Enabled {
                    self.interrupt_master_enable =
                        InterruptMasterEnable::EnableAfterNextInstruction;
                }
                OpResult::cycles(1)
            }
            Interrupt::Disable => {
                self.interrupt_master_enable = InterruptMasterEnable::Disabled;
                OpResult::cycles(1)
            }
            Interrupt::Await => {
                self.halted = true;
                OpResult::cycles(0)
            }
        }
    }
}
