pub struct Mbc5 {
    pub ram: Vec<[u8; 8 * 1024]>,
    pub ram_enabled: bool,
    pub rom_bank: u16,
    pub ram_bank: u8,
    pub rumble: bool,
}

impl Mbc5 {
    pub fn new(rom: &[u8], save_data: Option<Vec<u8>>) -> Self {
        Self::create(rom, save_data, false)
    }

    pub fn new_rumble(rom: &[u8], save_data: Option<Vec<u8>>) -> Self {
        Self::create(rom, save_data, true)
    }

    fn create(rom: &[u8], save_data: Option<Vec<u8>>, rumble: bool) -> Self {
        let mut ram = vec![[0u8; 8 * 1024]; super::num_ram_banks(rom)];
        if let Some(data) = &save_data {
            super::restore_banked(&mut ram, data);
        }

        Self {
            ram,
            ram_enabled: false,
            rom_bank: 1,
            ram_bank: 0,
            rumble,
        }
    }

    pub fn ram(&self) -> Option<Vec<u8>> {
        super::save_banked(&self.ram)
    }

    pub(super) fn switchable_rom_bank(&self) -> u16 {
        self.rom_bank
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

    pub fn write(&mut self, address: u16, value: u8) -> bool {
        match address {
            0x0000..=0x1fff => {
                self.ram_enabled = value & 0x0f == 0x0a;
                false
            }
            0x2000..=0x2fff => {
                self.rom_bank = (self.rom_bank & 0x100) | value as u16;
                false
            }
            0x3000..=0x3fff => {
                self.rom_bank = (self.rom_bank & 0xff) | ((value as u16 & 0x01) << 8);
                false
            }
            0x4000..=0x5fff => {
                self.ram_bank = if self.rumble {
                    value & 0x07
                } else {
                    value & 0x0f
                };
                false
            }
            0xa000..=0xbfff if self.ram_enabled => {
                let bank = self.ram_bank as usize;
                if bank < self.ram.len() {
                    self.ram[bank][(address - 0xa000) as usize] = value;
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}
