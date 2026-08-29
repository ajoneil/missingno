// Sachen's multicart mapper (Tauwasser's "Sachen MMC1"): a base-bank and
// mask pair windows each game's banks inside the multicart ROM, the header
// region reads through swapped address lines, and ROM A7 is forced high
// until an A15-edge count runs out — serving Sachen's logo to the boot ROM's
// copy and Nintendo's to its compare.
pub struct SachenMmc1 {
    pub base: u8,
    pub bank: u8,
    pub mask: u8,
    pub locked: bool,
    pub a15_falls: u8,
}

/// The unlock threshold: the boot ROM's 48 logo-copy reads, then one more —
/// the first compare read is served unlocked.
const A15_FALLS_TO_UNLOCK: u8 = 0x31;

/// Bank-register bits 5:4; both set opens the base and mask registers.
const MAP_ENABLE: u8 = 0x30;

/// Reads of 0x0100–0x01FF reach ROM through swapped lines: A0↔A6, A1↔A4.
pub(crate) fn header_window_permuted(address: u16) -> u16 {
    if address & 0xff00 != 0x0100 {
        return address;
    }
    (address & !0x53)
        | (address >> 6 & 0x01)
        | (address >> 3 & 0x02)
        | (address << 3 & 0x10)
        | (address << 6 & 0x40)
}

impl Default for SachenMmc1 {
    fn default() -> Self {
        Self::new()
    }
}

impl SachenMmc1 {
    pub fn new() -> Self {
        Self {
            base: 0x00,
            bank: 0x01,
            mask: 0x00,
            locked: true,
            a15_falls: 0,
        }
    }

    fn map_writes_enabled(&self) -> bool {
        self.bank & MAP_ENABLE == MAP_ENABLE
    }

    /// Mask bits come from the base register (the volume's window); the rest
    /// pass through from the game's own bank write.
    fn high_window_bank(&self) -> u8 {
        (self.bank & !self.mask) | (self.mask & self.base)
    }

    pub(super) fn switchable_rom_bank(&self) -> u16 {
        self.high_window_bank() as u16
    }

    /// An A15 high→low edge on the cartridge bus. The 0x31st unlocks before
    /// its access is served; only reset re-locks.
    pub fn a15_fell(&mut self) {
        if !self.locked {
            return;
        }
        self.a15_falls += 1;
        if self.a15_falls == A15_FALLS_TO_UNLOCK {
            self.locked = false;
        }
    }

    /// Boot has completed: the unlock count has long passed.
    pub fn boot_completed(&mut self) {
        self.locked = false;
    }

    /// The cartridge-edge /RESET: registers to reset values, lock re-armed.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn read(&self, rom: &[u8], address: u16) -> u8 {
        let (bank, in_bank) = match address {
            0x0000..=0x3fff => (self.mask & self.base, header_window_permuted(address)),
            0x4000..=0x7fff => (self.high_window_bank(), address - 0x4000),
            _ => return 0xff,
        };
        let in_bank = if self.locked { in_bank | 0x80 } else { in_bank };
        rom[(bank as usize * 0x4000 + in_bank as usize) % rom.len()]
    }

    pub fn write(&mut self, address: u16, value: u8) -> bool {
        match address {
            0x0000..=0x1fff if self.map_writes_enabled() => self.base = value,
            // Zero-adjusted over the whole byte: 0x00 stores 0x01, 0x80 stores
            // 0x80 and reaches bank 0 only through ROM-size aliasing.
            0x2000..=0x3fff => self.bank = if value == 0 { 1 } else { value },
            0x4000..=0x5fff if self.map_writes_enabled() => self.mask = value,
            _ => {}
        }
        false
    }
}
