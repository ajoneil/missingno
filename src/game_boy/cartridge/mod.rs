mod mbc;

use mbc::{
    MemoryBankController, huc1::Huc1, huc3::Huc3, mbc1::Mbc1, mbc2::Mbc2, mbc3::Mbc3, mbc5::Mbc5,
    mbc6::Mbc6, mbc7::Mbc7, no_mbc::NoMbc,
};

pub struct Cartridge {
    title: String,
    has_battery: bool,
    sgb_flag: bool,
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

        let sgb_flag = rom[0x146] == 0x03;
        let cartridge_type = rom[0x147];
        let has_battery = matches!(
            cartridge_type,
            0x03 | 0x06 | 0x09 | 0x10 | 0x13 | 0x1b | 0x1e | 0x22 | 0xfe | 0xff
        );
        let save = if has_battery { save_data } else { None };

        let mbc: Box<dyn MemoryBankController> = match cartridge_type {
            0x00 | 0x08 | 0x09 => Box::new(NoMbc::new(rom, save)),
            0x01..=0x03 => Box::new(Mbc1::new(rom, save)),
            0x05 | 0x06 => Box::new(Mbc2::new(rom, save)),
            0x0f..=0x13 => Box::new(Mbc3::new(rom, save)),
            0x19..=0x1b => Box::new(Mbc5::new(rom, save)),
            0x1c..=0x1e => Box::new(Mbc5::new_rumble(rom, save)),
            0x20 => Box::new(Mbc6::new(rom, save)),
            0x22 => Box::new(Mbc7::new(rom, save)),
            0xfe => Box::new(Huc3::new(rom, save)),
            0xff => Box::new(Huc1::new(rom, save)),

            _ => panic!("nyi: mbc {:2x}", cartridge_type),
        };

        Cartridge {
            title,
            has_battery,
            sgb_flag,
            mbc,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn has_battery(&self) -> bool {
        self.has_battery
    }

    pub fn supports_sgb(&self) -> bool {
        self.sgb_flag
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
