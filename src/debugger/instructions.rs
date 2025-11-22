use crate::game_boy::MemoryMapped;

pub struct InstructionsIterator<'a> {
    pub address: Option<u16>,
    pub memory: &'a MemoryMapped,
}

impl<'a> InstructionsIterator<'a> {
    pub fn new(address: u16, memory: &'a MemoryMapped) -> Self {
        InstructionsIterator {
            address: Some(address),
            memory,
        }
    }
}

impl<'a> Iterator for InstructionsIterator<'a> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(address) = self.address {
            self.address = Some(address + 1);
            Some(self.memory.read(address))
        } else {
            None
        }
    }
}
