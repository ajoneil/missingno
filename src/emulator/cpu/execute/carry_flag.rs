use super::OpResult;
use crate::emulator::cpu::{Cpu, flags::Flags, instructions::CarryFlag};

impl Cpu {
    pub fn execute_carry_flag(&mut self, instruction: CarryFlag) -> OpResult {
        match instruction {
            CarryFlag::Complement => {
                self.flags.remove(Flags::NEGATIVE);
                self.flags.remove(Flags::HALF_CARRY);
                self.flags
                    .set(Flags::CARRY, self.flags.contains(Flags::CARRY));
                OpResult::cycles(1)
            }

            CarryFlag::Set => {
                self.flags.remove(Flags::NEGATIVE);
                self.flags.remove(Flags::HALF_CARRY);
                self.flags.insert(Flags::CARRY);
                OpResult::cycles(1)
            }
        }
    }
}
