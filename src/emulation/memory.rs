use super::interrupts::{self, InterruptFlags};
use crate::emulation::Cartridge;

pub struct MemoryBus {
    cartridge: Cartridge,
    work_ram: [u8; 0x2000],
    high_ram: [u8; 0x80],
    interrupt_registers: interrupts::Registers,
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

impl MemoryBus {
    pub fn new(cartridge: Cartridge) -> Self {
        Self {
            cartridge,
            work_ram: [0; 0x2000],
            high_ram: [0; 0x80],
            interrupt_registers: interrupts::Registers {
                enabled: InterruptFlags::empty(),
                requested: InterruptFlags::empty(),
            },
        }
    }

    pub fn read(&self, address: u16) -> u8 {
        self.read_mapped(MappedAddress::map(address))
    }

    pub fn read_mapped(&self, address: MappedAddress) -> u8 {
        match address {
            MappedAddress::Cartridge(address) => self.cartridge.read(address),
            MappedAddress::WorkRam(address) => self.work_ram[address as usize],
            MappedAddress::HighRam(address) => self.high_ram[address as usize],
            MappedAddress::InterruptRegister(register) => match register {
                interrupts::Register::EnabledInterrupts => self.interrupt_registers.enabled.bits(),
                interrupts::Register::RequestedInterrupts => {
                    self.interrupt_registers.requested.bits()
                }
            },
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        self.write_mapped(MappedAddress::map(address), value);
    }

    pub fn write_mapped(&mut self, address: MappedAddress, value: u8) {
        match address {
            MappedAddress::Cartridge(_) => todo!(),
            MappedAddress::WorkRam(address) => self.work_ram[address as usize] = value,
            MappedAddress::HighRam(address) => self.work_ram[address as usize] = value,
            MappedAddress::InterruptRegister(register) => match register {
                interrupts::Register::EnabledInterrupts => {
                    self.interrupt_registers.enabled = InterruptFlags::from_bits_retain(value)
                }
                interrupts::Register::RequestedInterrupts => {
                    self.interrupt_registers.requested = InterruptFlags::from_bits_retain(value)
                }
            },
        }
    }

    pub fn cartridge(&self) -> &Cartridge {
        &self.cartridge
    }

    pub fn interrupt_registers(&self) -> &interrupts::Registers {
        &self.interrupt_registers
    }

    pub fn interrupt_registers_mut(&mut self) -> &mut interrupts::Registers {
        &mut self.interrupt_registers
    }
}
