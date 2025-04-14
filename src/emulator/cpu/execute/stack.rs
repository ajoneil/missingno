use super::OpResult;
use crate::emulator::cpu::{Cpu, cycles::Cycles, instructions::Stack};

impl Cpu {
    pub fn execute_stack(&mut self, instruction: Stack) -> OpResult {
        match instruction {
            Stack::Adjust(_) => todo!(),
            Stack::Push(source) => {
                let (value, _) = self.fetch16(source);
                let result = OpResult::write16(self.stack_pointer, value, Cycles(4));
                self.stack_pointer -= 2;
                result
            }
            Stack::Pop(_) => todo!(),
        }
    }
}
