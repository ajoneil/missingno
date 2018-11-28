use crate::mbc::no_mbc::NoMbc;
use crate::mbc::Mbc;
use crate::rom_info::MbcType;

pub struct Cartridge {
    rom: Vec<u8>,
    mbc: Box<Mbc>,
}

impl Cartridge {
    pub fn new(rom: Vec<u8>, mbc_type: MbcType) -> Cartridge {
        let mbc: Box<Mbc> = match mbc_type {
            MbcType::NoMBC => Box::new(NoMbc::new()),
            _ => panic!("Mbc not supported"),
        };

        Cartridge { rom: rom, mbc: mbc }
    }

    pub fn read(&self, address: u16) -> u8 {
        self.mbc.read(address, self.rom.as_slice())
    }
}
