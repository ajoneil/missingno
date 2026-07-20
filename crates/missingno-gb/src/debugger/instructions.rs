use crate::{Console, Model};

/// A byte-addressable memory the disassembler can read without side effects:
/// the live [`Console`] when paused, or a copied window around PC when the
/// core is running on the emulation thread. The 16-bit CPU-address view of the
/// core's `ReadMemory`.
pub trait ReadInstructionMemory {
    fn read(&self, address: u16) -> u8;
}

impl<M: Model> ReadInstructionMemory for Console<M> {
    fn read(&self, address: u16) -> u8 {
        Console::<M>::read(self, address)
    }
}

pub struct InstructionsIterator<'a, R: ReadInstructionMemory + ?Sized> {
    pub address: Option<u16>,
    pub memory: &'a R,
}

impl<'a, R: ReadInstructionMemory + ?Sized> InstructionsIterator<'a, R> {
    pub fn new(address: u16, memory: &'a R) -> Self {
        InstructionsIterator {
            address: Some(address),
            memory,
        }
    }
}

impl<R: ReadInstructionMemory + ?Sized> Iterator for InstructionsIterator<'_, R> {
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
