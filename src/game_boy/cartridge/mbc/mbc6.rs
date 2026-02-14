pub struct Mbc6 {
    flash: Vec<u8>,
    ram: Vec<[u8; 4 * 1024]>,
    ram_enabled: bool,
    flash_enabled: bool,
    rom_bank_a: u8,
    rom_bank_a_flash: bool,
    rom_bank_b: u8,
    rom_bank_b_flash: bool,
    ram_bank_a: u8,
    ram_bank_b: u8,
}

impl Mbc6 {
    pub fn new(_rom: &[u8], save_data: Option<Vec<u8>>) -> Self {
        let mut ram = vec![[0u8; 4 * 1024]; 8];
        if let Some(data) = &save_data {
            for (bank_idx, bank) in ram.iter_mut().enumerate() {
                let offset = bank_idx * 4 * 1024;
                if offset < data.len() {
                    let len = (data.len() - offset).min(bank.len());
                    bank[..len].copy_from_slice(&data[offset..offset + len]);
                }
            }
        }

        let flash = vec![0xff; 1024 * 1024];

        Self {
            flash,
            ram,
            ram_enabled: false,
            flash_enabled: false,
            rom_bank_a: 0,
            rom_bank_a_flash: false,
            rom_bank_b: 0,
            rom_bank_b_flash: false,
            ram_bank_a: 0,
            ram_bank_b: 0,
        }
    }

    fn read_rom_or_flash(&self, rom: &[u8], bank: u8, is_flash: bool, offset: usize) -> u8 {
        if is_flash && self.flash_enabled {
            let addr = bank as usize * 0x2000 + offset;
            if addr < self.flash.len() {
                self.flash[addr]
            } else {
                0xff
            }
        } else {
            let addr = bank as usize * 0x2000 + offset;
            if addr < rom.len() { rom[addr] } else { 0xff }
        }
    }

    pub fn ram(&self) -> Option<Vec<u8>> {
        Some(self.ram.iter().flatten().copied().collect())
    }

    pub fn read(&self, rom: &[u8], address: u16) -> u8 {
        match address {
            0x0000..=0x3fff => {
                if (address as usize) < rom.len() {
                    rom[address as usize]
                } else {
                    0xff
                }
            }
            0x4000..=0x5fff => self.read_rom_or_flash(
                rom,
                self.rom_bank_a,
                self.rom_bank_a_flash,
                (address - 0x4000) as usize,
            ),
            0x6000..=0x7fff => self.read_rom_or_flash(
                rom,
                self.rom_bank_b,
                self.rom_bank_b_flash,
                (address - 0x6000) as usize,
            ),
            0xa000..=0xafff if self.ram_enabled => {
                let bank = self.ram_bank_a as usize;
                if bank < self.ram.len() {
                    self.ram[bank][(address - 0xa000) as usize]
                } else {
                    0xff
                }
            }
            0xb000..=0xbfff if self.ram_enabled => {
                let bank = self.ram_bank_b as usize;
                if bank < self.ram.len() {
                    self.ram[bank][(address - 0xb000) as usize]
                } else {
                    0xff
                }
            }
            _ => 0xff,
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x03ff => self.ram_enabled = value & 0x0f == 0x0a,
            0x0400..=0x07ff => self.ram_bank_a = value & 0x07,
            0x0800..=0x0bff => self.ram_bank_b = value & 0x07,
            0x0c00..=0x0fff => self.flash_enabled = value & 0x01 != 0,
            0x1000..=0x1fff => {} // Flash write enable — ignored
            0x2000..=0x27ff => self.rom_bank_a = value,
            0x2800..=0x2fff => self.rom_bank_a_flash = value == 0x08,
            0x3000..=0x37ff => self.rom_bank_b = value,
            0x3800..=0x3fff => self.rom_bank_b_flash = value == 0x08,
            0xa000..=0xafff if self.ram_enabled => {
                let bank = self.ram_bank_a as usize;
                if bank < self.ram.len() {
                    self.ram[bank][(address - 0xa000) as usize] = value;
                }
            }
            0xb000..=0xbfff if self.ram_enabled => {
                let bank = self.ram_bank_b as usize;
                if bank < self.ram.len() {
                    self.ram[bank][(address - 0xb000) as usize] = value;
                }
            }
            _ => {}
        }
    }
}
