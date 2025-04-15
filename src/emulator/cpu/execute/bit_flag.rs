use super::OpResult;
use crate::emulator::{
    MemoryMapped,
    cpu::{Cpu, flags::Flags, instructions::BitFlag},
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
            BitFlag::Set(_, _) => todo!(),
            BitFlag::Unset(_, _) => todo!(),
        }
    }
}
