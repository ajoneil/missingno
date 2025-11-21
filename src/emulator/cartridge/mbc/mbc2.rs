use crate::emulator::cartridge::MemoryBankController;

pub struct Mbc2 {
    rom: Vec<u8>,
    ram: [u8; 0x200],
    ram_enabled: bool,
    bank: u8,
}

impl Mbc2 {
    pub fn new(rom: Vec<u8>) -> Self {
        Self {
            rom,
            ram: [0; 512],
            ram_enabled: false,
            bank: 0,
        }
    }

    fn current_bank(&self) -> u8 {
        (self.bank & 0xf).max(1)
    }
}

impl MemoryBankController for Mbc2 {
    fn rom(&self) -> &[u8] {
        &self.rom
    }

    fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x3fff => self.rom[address as usize],
            0x4000..=0x7fff => self.rom[(self.current_bank() as usize * 0x4000) + address as usize],
            0xa000..=0xbfff => self.ram[((address - 0xa000) % 0x200) as usize],
            _ => {
                unreachable!()
            }
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1ff => {
                if value & 0xf == 0xa {
                    self.ram_enabled = true
                }
            }
            0x2000..=0x3fff => {
                self.bank = value & 0xf;
            }

            _ => {}
        }
    }
}
