pub struct Cartridge {
    rom: Vec<u8>,
    title: String,
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

        Cartridge { rom, title }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn header_checksum(&self) -> u8 {
        self.rom[0x14d]
    }

    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    pub fn read(&self, address: u16) -> u8 {
        self.rom[address as usize]
    }

    // pub fn read(&self, address: u16) -> u8 {
    //     self.mbc.read(address, self.rom.as_slice())
    // }

    // pub fn write(&mut self, address: u16, val: u8) {
    //     self.mbc.write(address, val)
    // }
}

// impl Header {
//     pub fn new(rom: &[u8]) -> RomInfo {
//         let mbc_type = match rom[0x147] {
//             0x00 | 0x08 | 0x09 => MbcType::NoMBC,
//             0x01..=0x03 => MbcType::MBC1,
//             0x05 | 0x06 => MbcType::MBC2,
//             0x0b..=0x0d => MbcType::MMM01,
//             0x0f..=0x13 => MbcType::MBC3,
//             0x15..=0x17 => MbcType::MBC4,
//             0x19..=0x1e => MbcType::MBC5,
//             _ => MbcType::Unknown,
//         };

//         RomInfo {
//             title: title,
//             mbc_type: mbc_type,
//             checksum: rom[0x14d],
//         }
//     }
// }
