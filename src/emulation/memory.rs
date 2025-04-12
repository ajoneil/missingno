use super::{
    MemoryMapped,
    interrupts::{self, InterruptFlags},
};

pub struct Ram {
    pub work_ram: [u8; 0x2000],
    pub high_ram: [u8; 0x80],
}

impl Ram {
    pub fn new() -> Self {
        Self {
            work_ram: [0; 0x2000],
            high_ram: [0; 0x80],
        }
    }
}

pub enum MappedAddress {
    Cartridge(u16),
    WorkRam(u16),
    HighRam(u8),
    InterruptRegister(interrupts::Register),
}

impl MappedAddress {
    pub fn map(address: u16) -> Self {
        match address {
            0x0000..=0x7fff => Self::Cartridge(address),
            0xc000..=0xdfff => Self::WorkRam(address - 0xc000),
            0xff0f => Self::InterruptRegister(interrupts::Register::RequestedInterrupts),
            0xff80..=0xfffe => Self::HighRam((address - 0xff80) as u8),
            0xffff => Self::InterruptRegister(interrupts::Register::EnabledInterrupts),
            _ => todo!("Unimplemented write to {:04x}", address),
        }
    }
}

pub enum MemoryWrite {
    Write8(MappedAddress, u8),
    Write16((MappedAddress, u8), (MappedAddress, u8)),
}

impl MemoryMapped {
    pub fn read(&self, address: u16) -> u8 {
        self.read_mapped(MappedAddress::map(address))
    }

    pub fn read_mapped(&self, address: MappedAddress) -> u8 {
        match address {
            MappedAddress::Cartridge(address) => self.cartridge.read(address),
            MappedAddress::WorkRam(address) => self.ram.work_ram[address as usize],
            MappedAddress::HighRam(address) => self.ram.high_ram[address as usize],
            MappedAddress::InterruptRegister(register) => match register {
                interrupts::Register::EnabledInterrupts => self.interrupts.enabled.bits(),
                interrupts::Register::RequestedInterrupts => self.interrupts.requested.bits(),
            },
        }
    }

    pub fn write(&mut self, write: MemoryWrite) {
        match write {
            MemoryWrite::Write8(address, value) => self.write_mapped(address, value),
            MemoryWrite::Write16((address1, value1), (address2, value2)) => {
                self.write_mapped(address1, value1);
                self.write_mapped(address2, value2);
            }
        }
    }

    pub fn write_mapped(&mut self, address: MappedAddress, value: u8) {
        match address {
            MappedAddress::Cartridge(_) => todo!(),
            MappedAddress::WorkRam(address) => self.ram.work_ram[address as usize] = value,
            MappedAddress::HighRam(address) => self.ram.work_ram[address as usize] = value,
            MappedAddress::InterruptRegister(register) => match register {
                interrupts::Register::EnabledInterrupts => {
                    self.interrupts.enabled = InterruptFlags::from_bits_retain(value)
                }
                interrupts::Register::RequestedInterrupts => {
                    self.interrupts.requested = InterruptFlags::from_bits_retain(value)
                }
            },
        }
    }
}
