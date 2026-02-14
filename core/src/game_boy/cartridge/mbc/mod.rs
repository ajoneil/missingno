pub mod huc1;
pub mod huc3;
pub mod mbc1;
pub mod mbc2;
pub mod mbc3;
pub mod mbc5;
pub mod mbc6;
pub mod mbc7;
pub mod no_mbc;

pub enum Mbc {
    NoMbc(no_mbc::NoMbc),
    Mbc1(mbc1::Mbc1),
    Mbc2(mbc2::Mbc2),
    Mbc3(mbc3::Mbc3),
    Mbc5(mbc5::Mbc5),
    Mbc6(mbc6::Mbc6),
    Mbc7(mbc7::Mbc7),
    Huc1(huc1::Huc1),
    Huc3(huc3::Huc3),
}

impl Mbc {
    pub fn ram(&self) -> Option<Vec<u8>> {
        match self {
            Mbc::NoMbc(m) => m.ram(),
            Mbc::Mbc1(m) => m.ram(),
            Mbc::Mbc2(m) => m.ram(),
            Mbc::Mbc3(m) => m.ram(),
            Mbc::Mbc5(m) => m.ram(),
            Mbc::Mbc6(m) => m.ram(),
            Mbc::Mbc7(m) => m.ram(),
            Mbc::Huc1(m) => m.ram(),
            Mbc::Huc3(m) => m.ram(),
        }
    }

    pub fn read(&self, rom: &[u8], address: u16) -> u8 {
        match self {
            Mbc::NoMbc(m) => m.read(rom, address),
            Mbc::Mbc1(m) => m.read(rom, address),
            Mbc::Mbc2(m) => m.read(rom, address),
            Mbc::Mbc3(m) => m.read(rom, address),
            Mbc::Mbc5(m) => m.read(rom, address),
            Mbc::Mbc6(m) => m.read(rom, address),
            Mbc::Mbc7(m) => m.read(rom, address),
            Mbc::Huc1(m) => m.read(rom, address),
            Mbc::Huc3(m) => m.read(rom, address),
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match self {
            Mbc::NoMbc(m) => m.write(address, value),
            Mbc::Mbc1(m) => m.write(address, value),
            Mbc::Mbc2(m) => m.write(address, value),
            Mbc::Mbc3(m) => m.write(address, value),
            Mbc::Mbc5(m) => m.write(address, value),
            Mbc::Mbc6(m) => m.write(address, value),
            Mbc::Mbc7(m) => m.write(address, value),
            Mbc::Huc1(m) => m.write(address, value),
            Mbc::Huc3(m) => m.write(address, value),
        }
    }
}
