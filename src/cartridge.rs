use mbc::Mbc;
use mbc::no_mbc::NoMbc;
use rom_info::{MbcType, NoMBC};

pub struct Cartridge {
    rom: Vec<u8>,
    mbc: Box<Mbc>
}

impl Cartridge {
    pub fn new(rom: Vec<u8>, mbc_type: MbcType) -> Cartridge {
        let mbc : Box<Mbc> = match mbc_type {
            NoMBC => box NoMbc::new(),
            _ => fail!("Mbc not supported")
        };

        Cartridge {
            rom: rom,
            mbc: mbc
        }
    }
}
