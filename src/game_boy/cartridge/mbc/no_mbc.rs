use crate::game_boy::cartridge::MemoryBankController;

pub struct NoMbc {
    rom: Vec<u8>,
    ram: Option<[u8; 8 * 1024]>,
}

impl NoMbc {
    pub fn new(rom: Vec<u8>, save_data: Option<Vec<u8>>) -> Self {
        let ram = if rom[0x149] == 2 {
            let mut ram = [0; 8 * 1024];
            if let Some(data) = save_data {
                let len = data.len().min(ram.len());
                ram[..len].copy_from_slice(&data[..len]);
            }
            Some(ram)
        } else {
            None
        };

        Self { rom, ram }
    }
}

impl MemoryBankController for NoMbc {
    fn rom(&self) -> &[u8] {
        &self.rom
    }

    fn ram(&self) -> Option<Vec<u8>> {
        self.ram.map(|ram| ram.to_vec())
    }

    fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x7fff => self.rom[address as usize],
            0xa000..=0xbfff => match self.ram {
                Some(ram) => ram[(address - 0xa000) as usize],
                None => 0xff,
            },
            _ => 0xff,
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        if let Some(ram) = &mut self.ram {
            ram[(address - 0xa000) as usize] = value;
        }
    }
}
