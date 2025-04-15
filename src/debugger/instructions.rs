pub struct InstructionsIterator<'a> {
    pub address: u16,
    pub rom: &'a [u8],
}

impl<'a> Iterator for InstructionsIterator<'a> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.address >= self.rom.len() as u16 {
            return None;
        }

        let value = self.rom[self.address as usize];

        self.address += 1;
        // Skip over header as it's data and not opcodes
        if (0x104..0x14f).contains(&self.address) {
            self.address = 0x150;
        }

        Some(value)
    }
}
