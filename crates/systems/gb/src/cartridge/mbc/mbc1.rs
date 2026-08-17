// RAM lives inline — the mapper is the storage, not an indirection to it.
#[allow(clippy::large_enum_variant)]
pub enum Ram {
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

    /// The linear RAM size, all banks, matching [`Ram::to_vec`]'s layout.
    pub(super) fn len(&self) -> usize {
        match self {
            Ram::None => 0,
            Ram::Unbanked { data } => data.len(),
            Ram::Banked { data } => data.len() * data[0].len(),
        }
    }

    /// Restore from one linear span, ignoring the current bank and enable state
    /// — the write counterpart to [`Ram::peek`], filling banks in order.
    pub(super) fn restore(&mut self, src: &[u8]) {
        use super::{restore_banked, restore_flat};
        match self {
            Ram::None => {}
            Ram::Unbanked { data } => restore_flat(data, src),
            Ram::Banked { data } => restore_banked(data, src),
        }
    }

    /// A raw read at linear `offset`, ignoring the current bank and enable
    /// state; `0xFF` past the end.
    pub(super) fn peek(&self, offset: usize) -> u8 {
        match self {
            Ram::None => 0xff,
            Ram::Unbanked { data } => data.get(offset).copied().unwrap_or(0xff),
            Ram::Banked { data } => {
                let bank = offset / data[0].len();
                data.get(bank)
                    .and_then(|b| b.get(offset % b.len()).copied())
                    .unwrap_or(0xff)
            }
        }
    }
}

pub struct Mbc1 {
    pub ram: Ram,
    pub ram_enabled: bool,
    pub bank: u8,
    pub ram_bank: u8,
    pub mode1: bool,
    pub multicart: bool,
}

impl Mbc1 {
    pub fn new(rom: &[u8], save_data: Option<Vec<u8>>, multicart: bool) -> Self {
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

    pub(super) fn switchable_rom_bank(&self, rom_len: usize) -> u16 {
        self.current_bank(rom_len) as u16
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

    pub fn write(&mut self, address: u16, value: u8) -> bool {
        match address {
            0x0000..=0x1fff => {
                self.ram_enabled = value & 0xf == 0xa;
                false
            }
            0x2000..=0x3fff => {
                self.bank = value & 0x1f;
                false
            }
            0x4000..=0x5fff => {
                self.ram_bank = value & 0b11;
                false
            }
            0x6000..=0x7fff => {
                self.mode1 = value & 1 == 1;
                false
            }
            0xa000..=0xbfff if self.ram_enabled => {
                self.ram.write(address, value, self.effective_ram_bank());
                true
            }
            _ => false,
        }
    }
}

/// Detect MBC1M multicart ROMs by checking for a valid Nintendo logo at bank $10.
pub fn detect_multicart(rom: &[u8]) -> bool {
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

    rom[logo_offset..logo_offset + 48] == crate::cartridge::NINTENDO_LOGO
}
