pub struct Huc1 {
    ram: Vec<[u8; 8 * 1024]>,
    rom_bank: u8,
    ram_bank: u8,
    ir_mode: bool,
}

impl Huc1 {
    pub fn new(rom: &[u8], save_data: Option<Vec<u8>>) -> Self {
        let num_ram_banks = match rom[0x149] {
            2 => 1,
            3 => 4,
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
            rom_bank: 1,
            ram_bank: 0,
            ir_mode: false,
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
                let bank = self.rom_bank.max(1) as usize;
                let addr = bank * 0x4000 + (address - 0x4000) as usize;
                rom[addr % rom.len()]
            }
            0xa000..=0xbfff => {
                if self.ir_mode {
                    // No remote device — always report no light
                    0xc0
                } else {
                    let bank = self.ram_bank as usize;
                    if bank < self.ram.len() {
                        self.ram[bank][(address - 0xa000) as usize]
                    } else {
                        0xff
                    }
                }
            }
            _ => 0xff,
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x1fff => self.ir_mode = value == 0x0e,
            0x2000..=0x3fff => self.rom_bank = value & 0x3f,
            0x4000..=0x5fff => self.ram_bank = value & 0x03,
            0xa000..=0xbfff if !self.ir_mode => {
                let bank = self.ram_bank as usize;
                if bank < self.ram.len() {
                    self.ram[bank][(address - 0xa000) as usize] = value;
                }
            }
            _ => {}
        }
    }
}
