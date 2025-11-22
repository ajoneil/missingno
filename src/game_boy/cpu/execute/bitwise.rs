use super::OpResult;
use crate::game_boy::{
    Cpu, MemoryMapped,
    cpu::{self, instructions::Bitwise},
};

impl Cpu {
    pub fn execute_bitwise(&mut self, instruction: Bitwise, memory: &MemoryMapped) -> OpResult {
        match instruction {
            Bitwise::AndA(source) => {
                let (value, fetch_cycles) = self.fetch8(source, memory);
                self.a = self.a & value;

                self.flags = if self.a == 0 {
                    cpu::Flags::ZERO | cpu::Flags::HALF_CARRY
                } else {
                    cpu::Flags::HALF_CARRY
                };

                OpResult::cycles(1).add_cycles(fetch_cycles)
            }

            Bitwise::OrA(source) => {
                let (value, fetch_cycles) = self.fetch8(source, memory);
                self.a = self.a | value;

                self.flags = if self.a == 0 {
                    cpu::Flags::ZERO
                } else {
                    cpu::Flags::empty()
                };

                OpResult::cycles(1).add_cycles(fetch_cycles)
            }

            Bitwise::XorA(source) => {
                let (value, fetch_cycles) = self.fetch8(source, memory);
                self.a = self.a ^ value;

                self.flags = if self.a == 0 {
                    cpu::Flags::ZERO
                } else {
                    cpu::Flags::empty()
                };

                OpResult::cycles(1).add_cycles(fetch_cycles)
            }

            Bitwise::ComplementA => {
                self.a = !self.a;
                self.flags = self.flags | cpu::Flags::NEGATIVE | cpu::Flags::HALF_CARRY;
                OpResult::cycles(1)
            }
        }
    }
}
