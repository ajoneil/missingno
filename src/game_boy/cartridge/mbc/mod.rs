pub mod mbc1;
pub mod mbc2;
pub mod mbc3;
pub mod no_mbc;

pub trait MemoryBankController {
    fn rom(&self) -> &[u8];
    fn ram(&self) -> Option<Vec<u8>>;
    fn read(&self, address: u16) -> u8;
    fn write(&mut self, address: u16, value: u8);
}
