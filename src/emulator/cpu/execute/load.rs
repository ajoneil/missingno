use super::OpResult;
use crate::emulator::{
    Cpu, MemoryMapped,
    cpu::{cycles::Cycles, instructions::Load},
};

impl Cpu {
    pub fn execute_load(&mut self, instruction: Load, memory: &MemoryMapped) -> OpResult {
        match instruction {
            Load::Load8(target, source) => {
                let (value, fetch_cycles) = self.fetch8(source, memory);
                self.set8(target, value)
                    .add_cycles(fetch_cycles + Cycles(1))
            }

            Load::Load16(target, source) => {
                let (value, fetch_cycles) = self.fetch16(source);
                self.set16(target, value)
                    .add_cycles(fetch_cycles + Cycles(1))
            }
        }
    }
}
