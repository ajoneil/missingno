use crate::emulation::mbc::Mbc;

pub struct NoMbc;

impl NoMbc {
    pub fn new() -> NoMbc {
        NoMbc
    }
}

impl Mbc for NoMbc {
    fn read(&self, address: u16, rom: &[u8]) -> u8 {
        rom[address as usize]
    }

    fn write(&mut self, _address: u16, _val: u8) {
        // No MBC, so noop!
    }
}
