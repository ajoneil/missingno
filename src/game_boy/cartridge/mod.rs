mod mbc;

use mbc::{MemoryBankController, mbc1::Mbc1, mbc2::Mbc2, mbc3::Mbc3, no_mbc::NoMbc};

pub struct Cartridge {
    title: String,
    has_battery: bool,
    mbc: Box<dyn MemoryBankController>,
}

impl Cartridge {
    pub fn new(rom: Vec<u8>, save_data: Option<Vec<u8>>) -> Cartridge {
        let mut title = String::new();
        for character in rom[0x134..0x144].iter() {
            if *character == 0u8 {
                break;
            }

            title.push(*character as char)
        }

        let cartridge_type = rom[0x147];
        let has_battery = matches!(cartridge_type, 0x03 | 0x06 | 0x09 | 0x10 | 0x13);
        let save = if has_battery { save_data } else { None };

        let mbc: Box<dyn MemoryBankController> = match cartridge_type {
            0x00 | 0x08 | 0x09 => Box::new(NoMbc::new(rom, save)),
            0x01..=0x03 => Box::new(Mbc1::new(rom, save)),
            0x05 | 0x06 => Box::new(Mbc2::new(rom, save)),
            0x0f..=0x13 => Box::new(Mbc3::new(rom, save)),

            _ => panic!("nyi: mbc {:2x}", cartridge_type),
        };

        Cartridge {
            title,
            has_battery,
            mbc,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn has_battery(&self) -> bool {
        self.has_battery
    }

    pub fn ram(&self) -> Option<Vec<u8>> {
        self.mbc.ram()
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
