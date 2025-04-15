use super::OpResult;
use crate::emulator::{
    MemoryMapped,
    cpu::{Cpu, flags::Flags, instructions::BitShift},
};

impl Cpu {
    pub fn execute_bit_shift(&mut self, instruction: BitShift, memory: &MemoryMapped) -> OpResult {
        match instruction {
            BitShift::RotateA(_, _) => todo!(),
            BitShift::Rotate(_, _, _) => todo!(),
            BitShift::ShiftArithmetical(_, _) => todo!(),
            BitShift::ShiftRightLogical(_) => todo!(),
            BitShift::Swap(target) => {
                let (value, fetch_cycles) = self.fetch8(target.to_source(), memory);
                let new_value = value << 4 | (value >> 4 & 0xf);

                self.flags = if new_value == 0 {
                    Flags::ZERO
                } else {
                    Flags::empty()
                };

                self.set8(target, new_value).add_cycles(fetch_cycles)
            }
        }
    }
}
