pub mod no_mbc;

pub trait Mbc {
    fn read(address: u16, rom: &[u8]) -> u8;
}
