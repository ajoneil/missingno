use crate::game_boy::cartridge::MemoryBankController;
use crate::game_boy::save_state::Base64Bytes;

pub struct Mbc2 {
    rom: Vec<u8>,
    ram: [u8; 0x200],
    ram_enabled: bool,
    bank: u8,
}

impl Mbc2 {
    pub fn new(rom: Vec<u8>, save_data: Option<Vec<u8>>) -> Self {
        let mut ram = [0; 512];
        if let Some(data) = save_data {
            let len = data.len().min(ram.len());
            ram[..len].copy_from_slice(&data[..len]);
        }

        Self {
            rom,
            ram,
            ram_enabled: false,
            bank: 1,
        }
    }

    fn current_bank(&self) -> u8 {
        (self.bank & ((self.rom.len() / 0x4000) as u8 - 1)).max(1)
    }

    pub(crate) fn save_state(&self) -> crate::game_boy::save_state::MbcState {
        crate::game_boy::save_state::MbcState::Mbc2 {
            ram: Base64Bytes(self.ram.to_vec()),
            ram_enabled: self.ram_enabled,
            bank: self.bank,
        }
    }

    pub(crate) fn from_state(rom: Vec<u8>, state: crate::game_boy::save_state::MbcState) -> Self {
        let crate::game_boy::save_state::MbcState::Mbc2 {
            ram: ram_data,
            ram_enabled,
            bank,
        } = state
        else {
            unreachable!();
        };
        let mut ram = [0u8; 0x200];
        let len = ram_data.len().min(ram.len());
        ram[..len].copy_from_slice(&ram_data[..len]);
        Self {
            rom,
            ram,
            ram_enabled,
            bank,
        }
    }
}

impl MemoryBankController for Mbc2 {
    fn rom(&self) -> &[u8] {
        &self.rom
    }

    fn ram(&self) -> Option<Vec<u8>> {
        Some(self.ram.to_vec())
    }

    fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x3fff => self.rom[address as usize],
            0x4000..=0x7fff => {
                self.rom[(self.current_bank() as usize * 0x4000) + (address as usize - 0x4000)]
            }
            0xa000..=0xbfff => self.ram[((address - 0xa000) % 0x200) as usize],
            _ => {
                unreachable!()
            }
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x3fff => {
                if address & 0x100 == 0 {
                    self.ram_enabled = value & 0xf == 0xa;
                } else {
                    self.bank = value & 0xf;
                }
            }

            _ => {}
        }
    }
}
