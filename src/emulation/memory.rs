use crate::emulation::Cartridge;

pub struct MemoryBus {
    work_ram: [u8; 0x2000],
    cartridge: Cartridge,
}

pub enum MappedAddress {
    Cartridge(u16),
    WorkRam(u16),
}

impl MemoryBus {
    pub fn new(cartridge: Cartridge) -> Self {
        Self {
            work_ram: [0; 0x2000],
            cartridge,
        }
    }

    pub fn map_address(address: u16) -> MappedAddress {
        match address {
            0x0000..=0x7fff => MappedAddress::Cartridge(address),
            0xc000..=0xdfff => MappedAddress::WorkRam(address - 0xc000),
            _ => todo!(),
        }
    }

    pub fn read(&self, address: u16) -> u8 {
        match Self::map_address(address) {
            MappedAddress::Cartridge(address) => self.cartridge.read(address),
            MappedAddress::WorkRam(address) => self.work_ram[address as usize],
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match Self::map_address(address) {
            MappedAddress::WorkRam(address) => self.work_ram[address as usize] = value,
            _ => todo!(),
        }
    }

    pub fn cartridge(&self) -> &Cartridge {
        &self.cartridge
    }
}
