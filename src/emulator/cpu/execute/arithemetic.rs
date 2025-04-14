use super::OpResult;
use crate::emulator::{
    Cpu, MemoryMapped,
    cpu::{
        self,
        cycles::Cycles,
        flags::Flags,
        instructions::{Arithmetic, Arithmetic8, Arithmetic16},
        registers::Register16,
    },
};

impl Cpu {
    pub fn execute_arithmetic(
        &mut self,
        instruction: Arithmetic,
        memory: &MemoryMapped,
    ) -> OpResult {
        match instruction {
            Arithmetic::Arithmetic8(instruction) => match instruction {
                Arithmetic8::Increment(target) => {
                    let (original_value, fetch_cycles) = self.fetch8(target.to_source(), memory);
                    let value = original_value.wrapping_add(1);
                    let result = self.set8(target, value);

                    self.flags.set(cpu::Flags::ZERO, value == 0);
                    self.flags.insert(cpu::Flags::NEGATIVE);
                    // The half carry flag is set if we carry from bit 3 to 4
                    // i.e. xxxx1111 + 1 = xxxx0000
                    self.flags
                        .set(cpu::Flags::HALF_CARRY, value & 0b1111 == 0b0000);

                    result.add_cycles(Cycles(1) + fetch_cycles)
                }

                Arithmetic8::Decrement(target) => {
                    let (original_value, fetch_cycles) = self.fetch8(target.to_source(), memory);
                    let value = original_value.wrapping_sub(1);
                    let result = self.set8(target, value);

                    self.flags.set(cpu::Flags::ZERO, value == 0);
                    self.flags.insert(cpu::Flags::NEGATIVE);
                    // The half carry flag is set if we carry from bit 4 to 3
                    // i.e. xxx10000 - 1 = xxx01111
                    self.flags
                        .set(cpu::Flags::HALF_CARRY, value & 0b1111 == 0b1111);

                    result.add_cycles(Cycles(1) + fetch_cycles)
                }
                Arithmetic8::AddA(_) => todo!(),
                Arithmetic8::SubtractA(_) => todo!(),
                Arithmetic8::AddACarry(_) => todo!(),
                Arithmetic8::SubtractACarry(_) => todo!(),
                Arithmetic8::CompareA(source) => {
                    let (compare, fetch_cycles) = self.fetch8(source, memory);
                    let value = self.a.wrapping_sub(compare);
                    self.flags.set(cpu::Flags::ZERO, value == 0);
                    self.flags.insert(cpu::Flags::NEGATIVE);
                    self.flags
                        .set(cpu::Flags::HALF_CARRY, compare & 0xf > self.a & 0xf);
                    self.flags.set(cpu::Flags::CARRY, compare > self.a);

                    OpResult::cycles(1).add_cycles(fetch_cycles)
                }
            },

            Arithmetic::Arithmetic16(instruction) => match instruction {
                Arithmetic16::Increment(register) => {
                    self.set_register16(register, self.get_register16(register) + 1);
                    OpResult::cycles(2)
                }

                Arithmetic16::Decrement(register) => {
                    self.set_register16(register, self.get_register16(register) - 1);
                    OpResult::cycles(2)
                }

                Arithmetic16::AddHl(register) => {
                    let value = self.get_register16(register);
                    let hl = self.get_register16(Register16::Hl);

                    self.flags.remove(Flags::NEGATIVE);
                    self.flags
                        .set(Flags::HALF_CARRY, hl & 0xfff + value & 0xfff > 0xfff);
                    self.flags
                        .set(Flags::CARRY, hl & 0xff + value & 0xff > 0xff);

                    self.set_register16(Register16::Hl, hl + value);

                    OpResult::cycles(2)
                }
            },
        }
    }
}
