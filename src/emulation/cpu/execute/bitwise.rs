use crate::emulation::{
    Cpu, CpuFlags, MemoryBus,
    cpu::{cycles::Cycles, instructions::Bitwise},
};

impl Cpu {
    pub fn execute_bitwise(&mut self, instruction: Bitwise, memory_bus: &MemoryBus) -> Cycles {
        match instruction {
            Bitwise::AndA(source) => {
                let (value, fetch_cycles) = self.fetch8(source, memory_bus);
                self.a = self.a & value;

                self.flags = if self.a == 0 {
                    CpuFlags::ZERO & CpuFlags::HALF_CARRY
                } else {
                    CpuFlags::HALF_CARRY
                };

                Cycles(1) + fetch_cycles
            }

            Bitwise::OrA(source) => {
                let (value, fetch_cycles) = self.fetch8(source, memory_bus);
                self.a = self.a | value;

                self.flags = if self.a == 0 {
                    CpuFlags::ZERO
                } else {
                    CpuFlags::empty()
                };

                Cycles(1) + fetch_cycles
            }

            Bitwise::XorA(source) => {
                let (value, fetch_cycles) = self.fetch8(source, memory_bus);
                self.a = self.a ^ value;

                self.flags = if self.a == 0 {
                    CpuFlags::ZERO
                } else {
                    CpuFlags::empty()
                };

                Cycles(1) + fetch_cycles
            }

            Bitwise::ComplementA => {
                self.a = !self.a;
                self.flags = self.flags & CpuFlags::NEGATIVE & CpuFlags::HALF_CARRY;
                Cycles(1)
            }
        }
    }
}
