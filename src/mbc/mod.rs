pub mod no_mbc;

pub trait Mbc {
    fn read(&self, address: u16, rom: &[u8]) -> u8;
}
