use super::OpResult;
use crate::emulator::{
    MemoryMapped,
    cpu::{
        Cpu,
        flags::Flags,
        instructions::bit_shift::{BitShift, Direction},
    },
};

impl Cpu {
    pub fn execute_bit_shift(&mut self, instruction: BitShift, memory: &MemoryMapped) -> OpResult {
        match instruction {
            BitShift::RotateA(_, _) => todo!(),
            BitShift::Rotate(_, _, _) => todo!(),

            BitShift::ShiftArithmetical(direction, target) => {
                let (value, fetch_cycles) = self.fetch8(target.to_source(), memory);

                let new_value = match direction {
                    Direction::Left => {
                        self.flags.set(Flags::CARRY, value & 0b1000_0000 != 0);
                        value << 1
                    }

                    Direction::Right => {
                        self.flags.set(Flags::CARRY, value & 0b0000_0001 != 0);
                        value >> 1
                    }
                };

                self.flags.remove(Flags::NEGATIVE);
                self.flags.remove(Flags::HALF_CARRY);
                self.flags.set(Flags::ZERO, new_value == 0);

                self.set8(target, new_value).add_cycles(fetch_cycles)
            }

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
