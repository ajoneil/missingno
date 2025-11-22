use super::OpResult;
use crate::game_boy::{
    MemoryMapped,
    cpu::{Cpu, cycles::Cycles, instructions::Stack},
};

impl Cpu {
    pub fn execute_stack(&mut self, instruction: Stack, memory: &MemoryMapped) -> OpResult {
        match instruction {
            Stack::Adjust(_) => todo!(),

            Stack::Push(register) => {
                self.stack_pointer -= 2;
                OpResult::write16(self.stack_pointer, self.get_register16(register), Cycles(4))
            }

            Stack::Pop(register) => {
                self.set_register16(register, memory.read16(self.stack_pointer));
                self.stack_pointer += 2;

                OpResult::cycles(3)
            }
        }
    }
}
