use crate::cpu::instructions::instruction_length;
use crate::{Console, Model};

/// A byte-addressable memory the disassembler can read without side effects:
/// the live [`Console`] when paused, or a copied window around PC when the
/// core is running on the emulation thread.
pub trait ReadInstructionMemory {
    fn read(&self, address: u16) -> u8;
}

impl<M: Model> ReadInstructionMemory for Console<M> {
    fn read(&self, address: u16) -> u8 {
        Console::<M>::read(self, address)
    }
}

pub struct InstructionsIterator<'a, R: ReadInstructionMemory> {
    pub address: Option<u16>,
    pub memory: &'a R,
}

impl<'a, R: ReadInstructionMemory> InstructionsIterator<'a, R> {
    pub fn new(address: u16, memory: &'a R) -> Self {
        InstructionsIterator {
            address: Some(address),
            memory,
        }
    }
}

impl<R: ReadInstructionMemory> Iterator for InstructionsIterator<'_, R> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(address) = self.address {
            self.address = Some(address.wrapping_add(1));
            Some(self.memory.read(address))
        } else {
            None
        }
    }
}

/// Find instruction-aligned addresses before `pc` using backward sweep.
///
/// Tries disassembling forward from candidate start addresses before PC.
/// Returns addresses that produce an instruction stream landing exactly on PC,
/// giving up to `count` instructions of context before the current position.
pub fn addresses_before<R: ReadInstructionMemory>(pc: u16, count: usize, memory: &R) -> Vec<u16> {
    // Search back far enough to find `count` instructions.
    // Max instruction length is 3 bytes, so we need at most count*3 bytes back.
    let search_distance = (count * 3).min(128) as u16;
    let start = pc.saturating_sub(search_distance);

    // Try disassembling forward from `start`, collecting addresses that
    // form a valid instruction chain landing on PC.
    let mut best: Vec<u16> = Vec::new();

    // Try each possible starting offset
    for candidate in start..pc {
        let mut addr = candidate;
        let mut chain = Vec::new();

        // Walk forward, collecting instruction-aligned addresses
        while addr < pc {
            chain.push(addr);
            let opcode = memory.read(addr);
            addr = addr.wrapping_add(instruction_length(opcode));
        }

        // Only accept chains that land exactly on PC
        if addr == pc && chain.len() >= best.len() {
            best = chain;
        }
    }

    // Return only the last `count` addresses
    if best.len() > count {
        best.split_off(best.len() - count)
    } else {
        best
    }
}
