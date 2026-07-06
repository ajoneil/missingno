//! Cartridge ROM. The 6507 sees a 4 KB window; smaller ROMs mirror up.

pub enum Cartridge {
    Rom2K(Box<[u8; 0x800]>),
    Rom4K(Box<[u8; 0x1000]>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum CartridgeError {
    UnsupportedSize(usize),
}

impl Cartridge {
    pub fn load(rom: &[u8]) -> Result<Cartridge, CartridgeError> {
        match rom.len() {
            0x800 => Ok(Cartridge::Rom2K(Box::new(rom.try_into().unwrap()))),
            0x1000 => Ok(Cartridge::Rom4K(Box::new(rom.try_into().unwrap()))),
            size => Err(CartridgeError::UnsupportedSize(size)),
        }
    }

    pub fn read(&self, address: u16) -> u8 {
        match self {
            Cartridge::Rom2K(rom) => rom[(address & 0x7FF) as usize],
            Cartridge::Rom4K(rom) => rom[(address & 0xFFF) as usize],
        }
    }
}
