pub struct RomInfo {
    title: String,
    mbc: MbcType
}

pub enum MbcType {
    NoMBC,
    MBC1,
    MBC2,
    MMM01,
    MBC3,
    MBC4,
    MBC5,
    Unknown
}

impl RomInfo {
    pub fn new(rom: &[u8]) -> RomInfo {
         let mut title = String::new();
         for character in rom.slice(0x134, 0x144).iter() {
             if *character == 0u8 {
                 break;
             }

             title.push_char(*character as char)
         }

         let mbc = match rom[0x147] {
             0x00|0x08|0x09 => NoMBC,
             0x01..0x03 => MBC1,
             0x05|0x06 => MBC2,
             0x0b..0x0d => MMM01,
             0x0f..0x13 => MBC3,
             0x15..0x17 => MBC4,
             0x19..0x1e => MBC5,
             _ => Unknown
         };

         RomInfo {
             title: title,
             mbc: mbc
         }
    }

    pub fn title(&self) -> &str {
        self.title.as_slice()
    }
}
