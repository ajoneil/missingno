mod mbc;

use mbc::{
    Mbc, huc1::Huc1, huc3::Huc3, mbc1::Mbc1, mbc2::Mbc2, mbc3::Mbc3, mbc5::Mbc5, mbc6::Mbc6,
    mbc7::Mbc7, no_mbc::NoMbc,
};

pub struct Cartridge {
    title: String,
    has_battery: bool,
    sgb_flag: bool,
    rom: Vec<u8>,
    mbc: Mbc,
}

fn parse_title(rom: &[u8]) -> String {
    let mut title = String::new();
    for character in rom[0x134..0x144].iter() {
        if *character == 0u8 {
            break;
        }
        title.push(*character as char)
    }
    title
}

fn parse_header(rom: &[u8]) -> (String, bool, bool) {
    let title = parse_title(rom);
    let sgb_flag = rom[0x146] == 0x03;
    let cartridge_type = rom[0x147];
    let has_battery = matches!(
        cartridge_type,
        0x03 | 0x06 | 0x09 | 0x10 | 0x13 | 0x1b | 0x1e | 0x22 | 0xfe | 0xff
    );
    (title, sgb_flag, has_battery)
}

impl Cartridge {
    pub fn new(rom: Vec<u8>, save_data: Option<Vec<u8>>) -> Cartridge {
        let (title, sgb_flag, has_battery) = parse_header(&rom);
        let cartridge_type = rom[0x147];
        let save = if has_battery { save_data } else { None };

        let mbc = match cartridge_type {
            0x00 | 0x08 | 0x09 => Mbc::NoMbc(NoMbc::new(&rom, save)),
            0x01..=0x03 => Mbc::Mbc1(Mbc1::new(&rom, save)),
            0x05 | 0x06 => Mbc::Mbc2(Mbc2::new(&rom, save)),
            0x0f..=0x13 => Mbc::Mbc3(Mbc3::new(&rom, save)),
            0x19..=0x1b => Mbc::Mbc5(Mbc5::new(&rom, save)),
            0x1c..=0x1e => Mbc::Mbc5(Mbc5::new_rumble(&rom, save)),
            0x20 => Mbc::Mbc6(Mbc6::new(&rom, save)),
            0x22 => Mbc::Mbc7(Mbc7::new(&rom, save)),
            0xfe => Mbc::Huc3(Huc3::new(&rom, save)),
            0xff => Mbc::Huc1(Huc1::new(&rom, save)),

            _ => panic!("nyi: mbc {:2x}", cartridge_type),
        };

        Cartridge {
            title,
            has_battery,
            sgb_flag,
            rom,
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
        self.rom[0x14d]
    }

    pub fn global_checksum(&self) -> u16 {
        let hi = self.rom[0x14e] as u16;
        let lo = self.rom[0x14f] as u16;
        (hi << 8) | lo
    }

    pub fn read(&self, address: u16) -> u8 {
        self.mbc.read(&self.rom, address)
    }

    pub fn write(&mut self, address: u16, value: u8) {
        self.mbc.write(address, value);
    }
}
