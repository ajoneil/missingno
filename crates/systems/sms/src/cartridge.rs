//! Cartridge ROM behind the standard Sega mapper: three 16 KB slots with
//! the first kilobyte never paged (it holds the interrupt vectors).

pub struct Cartridge {
    rom: Vec<u8>,
    banks: [u8; 3],
}

#[derive(Debug, PartialEq, Eq)]
pub enum CartridgeError {
    /// ROMs are whole 16 KB pages (some dumps carry a 512-byte copier
    /// header, which callers strip first).
    UnsupportedSize(usize),
}

impl Cartridge {
    pub fn load(rom: &[u8]) -> Result<Cartridge, CartridgeError> {
        if rom.is_empty() || !rom.len().is_multiple_of(0x4000) {
            return Err(CartridgeError::UnsupportedSize(rom.len()));
        }
        Ok(Cartridge {
            rom: rom.to_vec(),
            banks: [0, 1, 2],
        })
    }

    pub fn read(&self, address: u16) -> u8 {
        let offset = if address < 0x0400 {
            address as usize
        } else {
            let slot = (address >> 14) as usize;
            let bank = self.banks[slot] as usize;
            bank * 0x4000 + (address & 0x3FFF) as usize
        };
        self.rom[offset % self.rom.len()]
    }

    /// A mapper latch write ($FFFD-$FFFF select the slot banks).
    pub fn write_bank(&mut self, slot: usize, bank: u8) {
        self.banks[slot] = bank;
    }
}
