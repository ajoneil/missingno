use crate::game_boy::cartridge::MemoryBankController;

enum Ram {
    None,
    Unbanked { data: [u8; 8 * 1024] },
    SimpleBanked { data: [[u8; 8 * 1024]; 4] },
    AdvancedBanked { data: [[u8; 8 * 1024]; 4] },
}

impl Ram {
    fn read(&self, address: u16, ram_bank: u8) -> u8 {
        let offset = (address - 0xa000) as usize;
        match self {
            Ram::None => 0xff,
            Ram::Unbanked { data } => data[offset],
            Ram::SimpleBanked { data } => data[0][offset],
            Ram::AdvancedBanked { data } => data[ram_bank as usize][offset],
        }
    }

    fn write(&mut self, address: u16, value: u8, ram_bank: u8) {
        let offset = (address - 0xa000) as usize;
        match self {
            Ram::None => {}
            Ram::Unbanked { data } => data[offset] = value,
            Ram::SimpleBanked { data } => data[0][offset] = value,
            Ram::AdvancedBanked { data } => data[ram_bank as usize][offset] = value,
        }
    }

    fn is_advanced(&self) -> bool {
        matches!(self, Ram::AdvancedBanked { .. })
    }

    fn set_advanced(&mut self, advanced: bool) {
        *self = match std::mem::replace(self, Ram::None) {
            Ram::SimpleBanked { data } if advanced => Ram::AdvancedBanked { data },
            Ram::AdvancedBanked { data } if !advanced => Ram::SimpleBanked { data },
            other => other,
        };
    }

    fn to_vec(&self) -> Option<Vec<u8>> {
        match self {
            Ram::None => None,
            Ram::Unbanked { data } => Some(data.to_vec()),
            Ram::SimpleBanked { data } | Ram::AdvancedBanked { data } => {
                Some(data.iter().flatten().copied().collect())
            }
        }
    }
}

pub struct Mbc1 {
    rom: Vec<u8>,
    ram: Ram,
    ram_enabled: bool,
    bank: u8,
    ram_bank: u8,
}

impl Mbc1 {
    pub fn new(rom: Vec<u8>, save_data: Option<Vec<u8>>) -> Self {
        let ram = match rom[0x149] {
            2 => {
                let mut data = [0; 8 * 1024];
                if let Some(save) = &save_data {
                    let len = save.len().min(data.len());
                    data[..len].copy_from_slice(&save[..len]);
                }
                Ram::Unbanked { data }
            }
            3 => {
                let mut data = [[0; 8 * 1024]; 4];
                if let Some(save) = &save_data {
                    for (bank_idx, bank) in data.iter_mut().enumerate() {
                        let offset = bank_idx * 8 * 1024;
                        if offset < save.len() {
                            let len = (save.len() - offset).min(bank.len());
                            bank[..len].copy_from_slice(&save[offset..offset + len]);
                        }
                    }
                }
                Ram::SimpleBanked { data }
            }
            _ => Ram::None,
        };

        Self {
            rom,
            ram,
            ram_enabled: false,
            bank: 0,
            ram_bank: 0,
        }
    }

    fn current_bank(&self) -> u8 {
        if self.rom.len() <= 512 * 1024 {
            (self.bank & 0x1f).max(1)
        } else {
            ((self.ram_bank << 5) | (self.bank & 0x1f)).max(1)
        }
    }
}

impl MemoryBankController for Mbc1 {
    fn rom(&self) -> &[u8] {
        &self.rom
    }

    fn ram(&self) -> Option<Vec<u8>> {
        self.ram.to_vec()
    }

    fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x3fff if self.ram.is_advanced() => {
                let bank = (self.ram_bank << 5) as usize;
                let addr = (bank * 0x4000 + address as usize) % self.rom.len();
                self.rom[addr]
            }
            0x0000..=0x3fff => self.rom[address as usize],
            0x4000..=0x7fff => {
                let bank = self.current_bank() as usize;
                let addr = (bank * 0x4000 + (address as usize - 0x4000)) % self.rom.len();
                self.rom[addr]
            }
            0xa000..=0xbfff if self.ram_enabled => self.ram.read(address, self.ram_bank),
            _ => 0xff,
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1fff => {
                self.ram_enabled = value & 0xf == 0xa;
            }
            0x2000..=0x3fff => {
                self.bank = value & 0x1f;
            }
            0x4000..=0x5fff => {
                self.ram_bank = value & 0b11;
            }
            0x6000..=0x7fff => {
                self.ram.set_advanced(value & 1 == 1);
            }
            0xa000..=0xbfff if self.ram_enabled => {
                self.ram.write(address, value, self.ram_bank);
            }
            _ => {}
        }
    }
}
