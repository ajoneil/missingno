use super::OpResult;
use crate::game_boy::{
    MemoryMapped,
    cpu::{Cpu, cycles::Cycles, flags::Flags, instructions::BitFlag},
};

impl Cpu {
    pub fn execute_bit_flag(&mut self, instruction: BitFlag, memory: &MemoryMapped) -> OpResult {
        match instruction {
            BitFlag::Check(bit, source) => {
                let (compare, fetch_cycles) = self.fetch8(source, memory);

                self.flags.set(Flags::ZERO, compare & (1 << bit) == 0);
                self.flags.remove(Flags::NEGATIVE);
                self.flags.insert(Flags::HALF_CARRY);

                OpResult::cycles(2).add_cycles(fetch_cycles)
            }

            BitFlag::Set(bit, target) => {
                let (value, fetch_cycles) = self.fetch8(target.to_source(), memory);
                let new_value = value | (1 << bit);

                self.set8(target, new_value)
                    .add_cycles(fetch_cycles)
                    .add_cycles(Cycles(2))
            }

            BitFlag::Unset(bit, target) => {
                let (value, fetch_cycles) = self.fetch8(target.to_source(), memory);
                let new_value = value ^ (1 << bit);

                self.set8(target, new_value)
                    .add_cycles(fetch_cycles)
                    .add_cycles(Cycles(2))
            }
        }
    }
}
