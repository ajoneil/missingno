mod mbc;

use mbc::{MemoryBankController, mbc1::Mbc1, mbc2::Mbc2, mbc3::Mbc3, no_mbc::NoMbc};

pub struct Cartridge {
    title: String,
    mbc: Box<dyn MemoryBankController>,
}

impl Cartridge {
    pub fn new(rom: Vec<u8>) -> Cartridge {
        let mut title = String::new();
        for character in rom[0x134..0x144].iter() {
            if *character == 0u8 {
                break;
            }

            title.push(*character as char)
        }

        let mbc: Box<dyn MemoryBankController> = match rom[0x147] {
            0x00 | 0x08 | 0x09 => Box::new(NoMbc::new(rom)),
            0x01..=0x03 => Box::new(Mbc1::new(rom)),
            0x05 | 0x06 => Box::new(Mbc2::new(rom)),
            0x0f..=0x13 => Box::new(Mbc3::new(rom)),

            _ => panic!("nyi: mbc {:2x}", rom[0x147]),
        };

        Cartridge { title, mbc }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn header_checksum(&self) -> u8 {
        self.mbc.rom()[0x14d]
    }

    pub fn read(&self, address: u16) -> u8 {
        self.mbc.read(address)
    }

    pub fn write(&mut self, address: u16, value: u8) {
        self.mbc.write(address, value);
    }
}
