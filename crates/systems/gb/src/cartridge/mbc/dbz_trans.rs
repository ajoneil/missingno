/// Mapper used by the unlicensed "GB DBZ GOKOU 2" cartridge. It behaves as MBC5
/// but adds a "half bank switch": the two 8 KB halves of the switchable-bank
/// window (`0x4000`-`0x5FFF` and `0x6000`-`0x7FFF`) can be pointed at different
/// half-banks independently, via writes whose low address byte is `0xD2`.
pub struct DbzTrans {
    pub ram: Vec<[u8; 8 * 1024]>,
    pub ram_enabled: bool,
    pub rom_bank: u16,
    pub ram_bank: u8,
    low_base: usize,
    high_base: usize,
    waiting_for_other_half: bool,
}

impl DbzTrans {
    pub fn new(rom: &[u8], save_data: Option<Vec<u8>>) -> Self {
        let mut ram = vec![[0u8; 8 * 1024]; super::num_ram_banks(rom)];
        if let Some(data) = &save_data {
            super::restore_banked(&mut ram, data);
        }

        Self {
            ram,
            ram_enabled: false,
            rom_bank: 1,
            ram_bank: 0,
            low_base: 0x4000,
            high_base: 0x6000,
            waiting_for_other_half: false,
        }
    }

    pub fn ram(&self) -> Option<Vec<u8>> {
        super::save_banked(&self.ram)
    }

    /// A normal bank write points both halves at the contiguous 16 KB bank.
    fn set_contiguous_bank(&mut self) {
        self.low_base = (self.rom_bank as usize) << 14;
        self.high_base = ((self.rom_bank as usize) << 14) + 0x2000;
    }

    /// Reload the summary state a save-state snapshot carries. The transient
    /// half-bank split is not snapshotted, so the window resets to contiguous.
    pub fn restore(&mut self, rom_bank: u16, ram_bank: u8, ram_enabled: bool) {
        self.rom_bank = rom_bank;
        self.ram_bank = ram_bank;
        self.ram_enabled = ram_enabled;
        self.waiting_for_other_half = false;
        self.set_contiguous_bank();
    }

    pub fn read(&self, rom: &[u8], address: u16) -> u8 {
        match address {
            0x0000..=0x3fff => rom[address as usize],
            0x4000..=0x5fff => rom[(self.low_base + (address - 0x4000) as usize) % rom.len()],
            0x6000..=0x7fff => rom[(self.high_base + (address - 0x6000) as usize) % rom.len()],
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
        // Half bank switch: `7?d2` remaps the upper half, then a following `2???`
        // (or the explicit `2?d2`) remaps the lower half.
        if address & 0xf0ff == 0x70d2 {
            self.high_base = ((value as usize) << 14) + 0x2000;
            self.waiting_for_other_half = true;
            return false;
        }
        if address & 0xf0ff == 0x20d2 || (self.waiting_for_other_half && address & 0xf000 == 0x2000)
        {
            self.low_base = (value as usize) << 14;
            self.waiting_for_other_half = false;
            return false;
        }

        match address {
            0x0000..=0x1fff => {
                self.ram_enabled = value & 0x0f == 0x0a;
                false
            }
            0x2000..=0x2fff => {
                self.rom_bank = (self.rom_bank & 0x100) | value as u16;
                self.set_contiguous_bank();
                false
            }
            0x3000..=0x3fff => {
                self.rom_bank = (self.rom_bank & 0xff) | ((value as u16 & 0x01) << 8);
                self.set_contiguous_bank();
                false
            }
            0x4000..=0x5fff => {
                self.ram_bank = value & 0x0f;
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
