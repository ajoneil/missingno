use super::OpResult;
use crate::game_boy::{
    MemoryMapped,
    cpu::{
        Cpu,
        cycles::Cycles,
        flags::Flags,
        instructions::bit_shift::{BitShift, Carry, Direction},
    },
};

impl Cpu {
    pub fn execute_bit_shift(&mut self, instruction: BitShift, memory: &MemoryMapped) -> OpResult {
        match instruction {
            BitShift::RotateA(direction, carry) => {
                let (new_value, new_carry) = self.rotate(self.a, direction, carry);

                self.flags = if new_carry {
                    Flags::CARRY
                } else {
                    Flags::empty()
                };

                self.a = new_value;
                OpResult::cycles(1)
            }

            BitShift::Rotate(direction, carry, target) => {
                let (value, fetch_cycles) = self.fetch8(target.to_source(), memory);

                let (new_value, new_carry) = self.rotate(value, direction, carry);

                self.flags.set(Flags::ZERO, new_value == 0);
                self.flags.set(Flags::CARRY, new_carry);
                self.flags.remove(Flags::NEGATIVE);
                self.flags.remove(Flags::HALF_CARRY);

                self.set8(target, new_value)
                    .add_cycles(fetch_cycles)
                    .add_cycles(Cycles(2))
            }

            BitShift::ShiftArithmetical(direction, target) => {
                let (value, fetch_cycles) = self.fetch8(target.to_source(), memory);

                let new_value = match direction {
                    Direction::Left => {
                        self.flags.set(Flags::CARRY, value & 0b1000_0000 != 0);
                        value << 1
                    }

                    Direction::Right => {
                        self.flags.set(Flags::CARRY, value & 0b0000_0001 != 0);
                        value >> 1 | (value & 0b1000_0000)
                    }
                };

                self.flags.remove(Flags::NEGATIVE);
                self.flags.remove(Flags::HALF_CARRY);
                self.flags.set(Flags::ZERO, new_value == 0);

                self.set8(target, new_value).add_cycles(fetch_cycles)
            }

            BitShift::ShiftRightLogical(target) => {
                let (value, fetch_cycles) = self.fetch8(target.to_source(), memory);

                let new_value = value >> 1;

                self.flags.set(Flags::CARRY, value & 0b0000_0001 != 0);
                self.flags.remove(Flags::NEGATIVE);
                self.flags.remove(Flags::HALF_CARRY);
                self.flags.set(Flags::ZERO, new_value == 0);

                self.set8(target, new_value).add_cycles(fetch_cycles)
            }

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

    fn rotate(&self, value: u8, direction: Direction, carry: Carry) -> (u8, bool) {
        match direction {
            Direction::Left => {
                let mut new_value = value << 1;
                let new_carry = value & 0b1000_0000 != 0;

                if match carry {
                    Carry::Through => self.flags.contains(Flags::CARRY),
                    Carry::SetOnly => new_carry,
                } {
                    new_value |= 1;
                }

                (new_value, new_carry)
            }

            Direction::Right => {
                let mut new_value = value >> 1;
                let new_carry = value & 0b1000_0000 != 0;

                if match carry {
                    Carry::Through => self.flags.contains(Flags::CARRY),
                    Carry::SetOnly => new_carry,
                } {
                    new_value |= 0b1000_0000;
                }

                (new_value, new_carry)
            }
        }
    }
}
