use crate::emulation::{
    Cpu, CpuFlags, MemoryBus,
    cpu::{
        cycles::Cycles,
        instructions::{Arithmetic, Arithmetic8},
    },
};

impl Cpu {
    pub fn execute_arithmetic(
        &mut self,
        instruction: Arithmetic,
        memory_bus: &mut MemoryBus,
    ) -> Cycles {
        match instruction {
            Arithmetic::Arithmetic8(instruction) => match instruction {
                Arithmetic8::Increment(target) => {
                    let (original_value, fetch_cycles) =
                        self.fetch8(target.to_source(), memory_bus);
                    let value = original_value.wrapping_add(1);
                    let set_cycles = self.set8(target, value, memory_bus);

                    self.flags.set(CpuFlags::ZERO, value == 0);
                    self.flags.insert(CpuFlags::NEGATIVE);
                    // The half carry flag is set if we carry from bit 3 to 4
                    // i.e. xxxx1111 + 1 = xxxx0000
                    self.flags
                        .set(CpuFlags::HALF_CARRY, value & 0b1111 == 0b0000);

                    Cycles(1) + fetch_cycles + set_cycles
                }

                Arithmetic8::Decrement(target) => {
                    let (original_value, fetch_cycles) =
                        self.fetch8(target.to_source(), memory_bus);
                    let value = original_value.wrapping_sub(1);
                    let set_cycles = self.set8(target, value, memory_bus);

                    self.flags.set(CpuFlags::ZERO, value == 0);
                    self.flags.insert(CpuFlags::NEGATIVE);
                    // The half carry flag is set if we carry from bit 4 to 3
                    // i.e. xxx10000 - 1 = xxx01111
                    self.flags
                        .set(CpuFlags::HALF_CARRY, value & 0b1111 == 0b1111);

                    Cycles(1) + fetch_cycles + set_cycles
                }
                Arithmetic8::AddA(_) => todo!(),
                Arithmetic8::SubtractA(_) => todo!(),
                Arithmetic8::AddACarry(_) => todo!(),
                Arithmetic8::SubtractACarry(_) => todo!(),
                Arithmetic8::CompareA(_) => todo!(),
            },
            Arithmetic::Arithmetic16(_) => todo!(),
        }
    }
}
