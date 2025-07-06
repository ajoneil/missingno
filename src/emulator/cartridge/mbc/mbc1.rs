use crate::emulator::cartridge::MemoryBankController;

enum Ram {
    None,
    Unbanked([u8; 8 * 1024]),
    Banked([[u8; 8 * 1024]; 4]),
}

enum BankingMode {
    Simple,
    Advanced,
}

pub struct Mbc1 {
    rom: Vec<u8>,
    ram: Ram,
    ram_enabled: bool,
    bank: u8,
    ram_bank: u8,
    banking_mode: BankingMode,
}

impl Mbc1 {
    pub fn new(rom: Vec<u8>) -> Self {
        let ram = match rom[0x149] {
            2 => Ram::Unbanked([0; 8 * 1024]),
            3 => Ram::Banked([[0; 8 * 1024]; 4]),
            _ => Ram::None,
        };

        Self {
            rom,
            ram,
            ram_enabled: false,
            bank: 0,
            ram_bank: 0,
            banking_mode: BankingMode::Simple,
        }
    }

    fn current_bank(&self) -> u8 {
        if self.rom.len() <= 512 * 1024 {
            (self.bank & 0x1f).max(1)
        } else {
            panic!("nyi")
        }
    }
}

impl MemoryBankController for Mbc1 {
    fn rom(&self) -> &[u8] {
        &self.rom
    }

    fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x3fff => match self.banking_mode {
                BankingMode::Simple => self.rom[address as usize],
                BankingMode::Advanced => panic!("nyi"),
            },
            0x4000..=0x7fff => match self.banking_mode {
                BankingMode::Simple => {
                    self.rom[(self.current_bank() as usize * 0x4000) + address as usize]
                }
                BankingMode::Advanced => panic!("nyi"),
            },

            0xa000..=0xbfff => match self.ram {
                Ram::Unbanked(ram) => ram[(address - 0xa000) as usize],
                Ram::Banked(ram) => ram[self.ram_bank as usize][(address - 0xa000) as usize],
                Ram::None => 0xff,
            },
            _ => 0xff,
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
                self.bank = value & 0x1f;
            }
            0x4000..=0x5fff => {
                self.ram_bank = value & 0b11;
            }
            0x6000..=0x7fff => {
                self.banking_mode = if value & 1 == 1 {
                    BankingMode::Advanced
                } else {
                    BankingMode::Simple
                };
            }

            _ => {}
        }
    }
}
