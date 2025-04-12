use crate::emulation::{
    Cpu, MemoryBus,
    cpu::{cycles::Cycles, instructions::Load},
};

impl Cpu {
    pub fn execute_load(&mut self, instruction: Load, memory_bus: &MemoryBus) -> Cycles {
        match instruction {
            Load::Load8(target, source) => {
                let (value, fetch_cycles) = self.fetch8(source, memory_bus);
                let set_cycles = self.set8(target, value);

                Cycles(1) + fetch_cycles + set_cycles
            }

            Load::Load16(target, source) => {
                let (value, fetch_cycles) = self.fetch16(source);
                let set_cycles = self.set16(target, value);

                Cycles(1) + fetch_cycles + set_cycles
            }
        }
    }
}
