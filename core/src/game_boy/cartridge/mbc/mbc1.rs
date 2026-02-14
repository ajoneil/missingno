enum Ram {
    None,
    Unbanked { data: [u8; 8 * 1024] },
    Banked { data: [[u8; 8 * 1024]; 4] },
}

impl Ram {
    fn read(&self, address: u16, bank: u8) -> u8 {
        let offset = (address - 0xa000) as usize;
        match self {
            Ram::None => 0xff,
            Ram::Unbanked { data } => data[offset],
            Ram::Banked { data } => data[bank as usize][offset],
        }
    }

    fn write(&mut self, address: u16, value: u8, bank: u8) {
        let offset = (address - 0xa000) as usize;
        match self {
            Ram::None => {}
            Ram::Unbanked { data } => data[offset] = value,
            Ram::Banked { data } => data[bank as usize][offset] = value,
        }
    }

    fn to_vec(&self) -> Option<Vec<u8>> {
        match self {
            Ram::None => None,
            Ram::Unbanked { data } => Some(data.to_vec()),
            Ram::Banked { data } => Some(data.iter().flatten().copied().collect()),
        }
    }
}

pub struct Mbc1 {
    ram: Ram,
    ram_enabled: bool,
    bank: u8,
    ram_bank: u8,
    mode1: bool,
    multicart: bool,
}

impl Mbc1 {
    pub fn new(rom: &[u8], save_data: Option<Vec<u8>>) -> Self {
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
                Ram::Banked { data }
            }
            _ => Ram::None,
        };

        let multicart = detect_multicart(rom);

        Self {
            ram,
            ram_enabled: false,
            bank: 0,
            ram_bank: 0,
            mode1: false,
            multicart,
        }
    }

    fn current_bank(&self, rom_len: usize) -> u8 {
        if self.multicart {
            // MBC1M: BANK2 applies to bits 4-5, only lower 4 bits of BANK1 used.
            // The 0→1 check uses the full 5-bit register, not the masked 4-bit value.
            if self.bank & 0x1f == 0 {
                (self.ram_bank << 4) | 1
            } else {
                (self.ram_bank << 4) | (self.bank & 0x0f)
            }
        } else if rom_len <= 512 * 1024 {
            let bank1 = self.bank & 0x1f;
            if bank1 == 0 { 1 } else { bank1 }
        } else {
            let bank1 = self.bank & 0x1f;
            let bank1 = if bank1 == 0 { 1 } else { bank1 };
            (self.ram_bank << 5) | bank1
        }
    }

    fn zero_bank(&self) -> u8 {
        if self.multicart {
            self.ram_bank << 4
        } else {
            self.ram_bank << 5
        }
    }

    /// In mode 0, RAM always uses bank 0. In mode 1, RAM uses the selected bank.
    fn effective_ram_bank(&self) -> u8 {
        if self.mode1 { self.ram_bank } else { 0 }
    }

    pub fn ram(&self) -> Option<Vec<u8>> {
        self.ram.to_vec()
    }

    pub fn read(&self, rom: &[u8], address: u16) -> u8 {
        match address {
            0x0000..=0x3fff if self.mode1 => {
                let bank = self.zero_bank() as usize;
                let addr = (bank * 0x4000 + address as usize) % rom.len();
                rom[addr]
            }
            0x0000..=0x3fff => rom[address as usize],
            0x4000..=0x7fff => {
                let bank = self.current_bank(rom.len()) as usize;
                let addr = (bank * 0x4000 + (address as usize - 0x4000)) % rom.len();
                rom[addr]
            }
            0xa000..=0xbfff if self.ram_enabled => {
                self.ram.read(address, self.effective_ram_bank())
            }
            _ => 0xff,
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
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
                self.mode1 = value & 1 == 1;
            }
            0xa000..=0xbfff if self.ram_enabled => {
                self.ram.write(address, value, self.effective_ram_bank());
            }
            _ => {}
        }
    }
}

/// Detect MBC1M multicart ROMs by checking for a valid Nintendo logo at bank $10.
fn detect_multicart(rom: &[u8]) -> bool {
    // Only 1 MiB ROMs can be MBC1M multicarts
    if rom.len() != 1024 * 1024 {
        return false;
    }

    // Nintendo logo at bank $10, offset 0x104-0x133
    let bank10_base = 0x10 * 0x4000;
    let logo_offset = bank10_base + 0x104;
    if logo_offset + 48 > rom.len() {
        return false;
    }

    const NINTENDO_LOGO: [u8; 48] = [
        0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00,
        0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD,
        0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB,
        0xB9, 0x33, 0x3E,
    ];

    rom[logo_offset..logo_offset + 48] == NINTENDO_LOGO
}
