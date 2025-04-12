use super::OpResult;
use crate::emulation::{Cpu, CpuFlags, MemoryMapped, cpu::instructions::Bitwise};

impl Cpu {
    pub fn execute_bitwise(&mut self, instruction: Bitwise, memory: &MemoryMapped) -> OpResult {
        match instruction {
            Bitwise::AndA(source) => {
                let (value, fetch_cycles) = self.fetch8(source, memory);
                self.a = self.a & value;

                self.flags = if self.a == 0 {
                    CpuFlags::ZERO & CpuFlags::HALF_CARRY
                } else {
                    CpuFlags::HALF_CARRY
                };

                OpResult::cycles(1).add_cycles(fetch_cycles)
            }

            Bitwise::OrA(source) => {
                let (value, fetch_cycles) = self.fetch8(source, memory);
                self.a = self.a | value;

                self.flags = if self.a == 0 {
                    CpuFlags::ZERO
                } else {
                    CpuFlags::empty()
                };

                OpResult::cycles(1).add_cycles(fetch_cycles)
            }

            Bitwise::XorA(source) => {
                let (value, fetch_cycles) = self.fetch8(source, memory);
                self.a = self.a ^ value;

                self.flags = if self.a == 0 {
                    CpuFlags::ZERO
                } else {
                    CpuFlags::empty()
                };

                OpResult::cycles(1).add_cycles(fetch_cycles)
            }

            Bitwise::ComplementA => {
                self.a = !self.a;
                self.flags = self.flags & CpuFlags::NEGATIVE & CpuFlags::HALF_CARRY;
                OpResult::cycles(1)
            }
        }
    }
}
