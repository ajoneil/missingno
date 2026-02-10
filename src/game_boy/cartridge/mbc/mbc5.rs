use crate::game_boy::save_state::Base64Bytes;

pub struct Mbc5 {
    ram: Vec<[u8; 8 * 1024]>,
    ram_enabled: bool,
    rom_bank: u16,
    ram_bank: u8,
    rumble: bool,
}

impl Mbc5 {
    pub fn new(rom: &[u8], save_data: Option<Vec<u8>>) -> Self {
        Self::create(rom, save_data, false)
    }

    pub fn new_rumble(rom: &[u8], save_data: Option<Vec<u8>>) -> Self {
        Self::create(rom, save_data, true)
    }

    fn create(rom: &[u8], save_data: Option<Vec<u8>>, rumble: bool) -> Self {
        let num_ram_banks = match rom[0x149] {
            2 => 1,
            3 => 4,
            4 => 16,
            5 => 8,
            _ => 0,
        };

        let mut ram = vec![[0u8; 8 * 1024]; num_ram_banks];
        if let Some(data) = &save_data {
            for (bank_idx, bank) in ram.iter_mut().enumerate() {
                let offset = bank_idx * 8 * 1024;
                if offset < data.len() {
                    let len = (data.len() - offset).min(bank.len());
                    bank[..len].copy_from_slice(&data[offset..offset + len]);
                }
            }
        }

        Self {
            ram,
            ram_enabled: false,
            rom_bank: 1,
            ram_bank: 0,
            rumble,
        }
    }

    pub(crate) fn save_state(&self) -> crate::game_boy::save_state::MbcState {
        crate::game_boy::save_state::MbcState::Mbc5 {
            ram: Base64Bytes::from_banks(&self.ram),
            ram_enabled: self.ram_enabled,
            rom_bank: self.rom_bank,
            ram_bank: self.ram_bank,
        }
    }

    pub(crate) fn from_state(rom: &[u8], state: crate::game_boy::save_state::MbcState) -> Self {
        let crate::game_boy::save_state::MbcState::Mbc5 {
            ram: ram_data,
            ram_enabled,
            rom_bank,
            ram_bank,
        } = state
        else {
            unreachable!();
        };
        let rumble = matches!(rom[0x147], 0x1c..=0x1e);
        let num_ram_banks = match rom[0x149] {
            2 => 1,
            3 => 4,
            4 => 16,
            5 => 8,
            _ => 0,
        };
        Self {
            ram: ram_data.into_banks(num_ram_banks),
            ram_enabled,
            rom_bank,
            ram_bank,
            rumble,
        }
    }

    pub fn ram(&self) -> Option<Vec<u8>> {
        if self.ram.is_empty() {
            None
        } else {
            Some(self.ram.iter().flatten().copied().collect())
        }
    }

    pub fn read(&self, rom: &[u8], address: u16) -> u8 {
        match address {
            0x0000..=0x3fff => rom[address as usize],
            0x4000..=0x7fff => {
                let addr = self.rom_bank as usize * 0x4000 + (address - 0x4000) as usize;
                rom[addr % rom.len()]
            }
            0xa000..=0xbfff if self.ram_enabled => {
                let bank = self.ram_bank as usize;
                if bank < self.ram.len() {
                    self.ram[bank][(address - 0xa000) as usize]
                } else {
                    0xff
                }
            }
            _ => 0xff,
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1fff => self.ram_enabled = value & 0x0f == 0x0a,
            0x2000..=0x2fff => {
                self.rom_bank = (self.rom_bank & 0x100) | value as u16;
            }
            0x3000..=0x3fff => {
                self.rom_bank = (self.rom_bank & 0xff) | ((value as u16 & 0x01) << 8);
            }
            0x4000..=0x5fff => {
                self.ram_bank = if self.rumble {
                    value & 0x07
                } else {
                    value & 0x0f
                };
            }
            0xa000..=0xbfff if self.ram_enabled => {
                let bank = self.ram_bank as usize;
                if bank < self.ram.len() {
                    self.ram[bank][(address - 0xa000) as usize] = value;
                }
            }
            _ => {}
        }
    }
}
