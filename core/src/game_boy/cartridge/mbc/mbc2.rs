pub struct Mbc2 {
    ram: [u8; 0x200],
    ram_enabled: bool,
    bank: u8,
}

impl Mbc2 {
    pub fn new(_rom: &[u8], save_data: Option<Vec<u8>>) -> Self {
        let mut ram = [0; 512];
        if let Some(data) = save_data {
            let len = data.len().min(ram.len());
            ram[..len].copy_from_slice(&data[..len]);
        }

        Self {
            ram,
            ram_enabled: false,
            bank: 1,
        }
    }

    fn current_bank(&self, rom_len: usize) -> u8 {
        (self.bank & ((rom_len / 0x4000) as u8 - 1)).max(1)
    }

    pub fn ram(&self) -> Option<Vec<u8>> {
        Some(self.ram.to_vec())
    }

    pub fn read(&self, rom: &[u8], address: u16) -> u8 {
        match address {
            0x0000..=0x3fff => rom[address as usize],
            0x4000..=0x7fff => {
                rom[(self.current_bank(rom.len()) as usize * 0x4000) + (address as usize - 0x4000)]
            }
            0xa000..=0xbfff => self.ram[((address - 0xa000) % 0x200) as usize],
            _ => {
                unreachable!()
            }
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
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
