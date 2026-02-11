use super::OpResult;
use crate::game_boy::{
    MemoryMapped,
    cpu::{Cpu, cycles::Cycles, flags::Flags, instructions::Stack},
};

impl Cpu {
    pub fn execute_stack(&mut self, instruction: Stack, memory: &MemoryMapped) -> OpResult {
        match instruction {
            Stack::Adjust(offset) => {
                let sp = self.stack_pointer;
                let offset_u8 = offset as u8;
                self.stack_pointer = sp.wrapping_add(offset as i16 as u16);

                self.flags.remove(Flags::ZERO);
                self.flags.remove(Flags::NEGATIVE);
                self.flags.set(
                    Flags::HALF_CARRY,
                    (sp & 0xf) + (offset_u8 as u16 & 0xf) > 0xf,
                );
                self.flags
                    .set(Flags::CARRY, (sp & 0xff) + (offset_u8 as u16 & 0xff) > 0xff);

                OpResult::cycles(4)
            }

            Stack::Push(register) => {
                self.stack_pointer = self.stack_pointer.wrapping_sub(2);
                OpResult::write16(self.stack_pointer, self.get_register16(register), Cycles(4))
            }

            Stack::Pop(register) => {
                self.set_register16(register, memory.read16(self.stack_pointer));
                self.stack_pointer = self.stack_pointer.wrapping_add(2);

                OpResult::cycles(3)
            }
        }
    }
}
