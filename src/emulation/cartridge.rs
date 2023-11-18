use super::mbc::no_mbc::NoMbc;
use super::mbc::Mbc;
use super::rom_info::MbcType;

pub struct Cartridge {
    rom: Vec<u8>,
    mbc: Box<dyn Mbc>,
}

impl Cartridge {
    pub fn new(rom: Vec<u8>, mbc_type: MbcType) -> Cartridge {
        let mbc: Box<dyn Mbc> = match mbc_type {
            MbcType::NoMBC => Box::new(NoMbc::new()),
            _ => {
                println!("Mbc {:?} not supported, continuing anyway..", mbc_type);
                Box::new(NoMbc::new())
            }
        };

        Cartridge { rom, mbc }
    }

    pub fn read(&self, address: u16) -> u8 {
        self.mbc.read(address, self.rom.as_slice())
    }

    pub fn write(&mut self, address: u16, val: u8) {
        self.mbc.write(address, val)
    }
}
