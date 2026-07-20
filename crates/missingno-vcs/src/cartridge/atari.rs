//! The hotspot-select family (F8/F6/F4, and the homebrew EF/DF/BF that scale it
//! up), optionally with Superchip cart RAM.
//!
//! A run of fixed addresses near the top of the window are hotspots: any bus
//! access landing on one selects its 4 KB bank, and the data bus is irrelevant.
//! The cart decodes 13 lines, so the CPU reaches them through their $FFxx
//! mirror. What separates the boards is only where the run starts and how many
//! banks it covers — F8 spends two hotspots, BF sixty-four — never the
//! mechanism, so they share it.
//!
//! Superchip (SARA) adds 128 bytes of static RAM under the bottom of the
//! window. The cart edge has no read/write strobe, so the RAM splits into two
//! ports: the low half is the write port (an access latches the data bus), the
//! high half is the read port. The RAM sits outside the banked ROM, so a bank
//! switch never disturbs it, and it shadows the ROM bytes under it.

pub const F8_HOTSPOT_BASE: u16 = 0x1FF8;
pub const F6_HOTSPOT_BASE: u16 = 0x1FF6;
pub const F4_HOTSPOT_BASE: u16 = 0x1FF4;
/// The homebrew boards need a wider run, so theirs starts lower.
pub const EF_HOTSPOT_BASE: u16 = 0x1FE0;
pub const DF_HOTSPOT_BASE: u16 = 0x1FC0;
pub const BF_HOTSPOT_BASE: u16 = 0x1F80;

/// Nothing on a board latches a bank at reset, so the bank it wakes in is
/// undefined state and most images force one before looking. DF images instead
/// expect the bank both references settle on.
pub const DF_START_BANK: usize = 15;

pub const SUPERCHIP_RAM_SIZE: usize = 0x80;

const BANK_SIZE: usize = 0x1000;

pub struct Atari {
    image: Vec<u8>,
    bank: usize,
    banks: usize,
    hotspot_base: u16,
    ram: Option<Box<[u8; SUPERCHIP_RAM_SIZE]>>,
}

impl Atari {
    pub fn new(rom: &[u8], hotspot_base: u16, superchip: bool) -> Atari {
        Atari {
            image: rom.to_vec(),
            bank: 0,
            banks: rom.len() / BANK_SIZE,
            hotspot_base,
            ram: superchip.then(|| Box::new([0; SUPERCHIP_RAM_SIZE])),
        }
    }

    /// Wake in a bank other than the first.
    pub fn waking_in(mut self, bank: usize) -> Atari {
        self.bank = bank;
        self
    }

    /// The board decodes 13 lines; one hotspot per bank, counting up from the
    /// board's base address.
    fn hotspot(&mut self, address: u16) {
        let decoded = address & 0x1FFF;
        let offset = decoded.wrapping_sub(self.hotspot_base) as usize;
        if decoded >= self.hotspot_base && offset < self.banks {
            self.bank = offset;
        }
    }

    pub fn read(&mut self, address: u16, bus: u8) -> u8 {
        self.hotspot(address);
        if let Some(ram) = &mut self.ram {
            let offset = (address & 0x0FFF) as usize;
            // The cart slot has no R/W line: a write-port read still stores,
            // latching the floating bus byte the CPU also sees.
            if offset < SUPERCHIP_RAM_SIZE {
                ram[offset] = bus;
                return bus;
            }
        }
        self.peek(address)
    }

    pub fn write_access(&mut self, address: u16, data: u8) {
        self.hotspot(address);
        if let Some(ram) = &mut self.ram {
            let offset = (address & 0x0FFF) as usize;
            if offset < SUPERCHIP_RAM_SIZE {
                ram[offset] = data;
            }
        }
    }

    /// The Superchip cart RAM, all of it; empty on a board without one.
    pub(super) fn ram(&self) -> &[u8] {
        match &self.ram {
            Some(ram) => ram.as_slice(),
            None => &[],
        }
    }

    /// The full ROM image, all banks in file order, for the debugger's
    /// bank-complete `rom` region.
    pub(super) fn rom(&self) -> &[u8] {
        &self.image
    }

    /// The 4 KB bank currently paged into the window, for the debugger.
    pub(super) fn selected_bank(&self) -> usize {
        self.bank
    }

    /// Re-page the window to a saved bank, clamped to the board's bank count.
    pub(super) fn set_bank(&mut self, bank: usize) {
        if bank < self.banks {
            self.bank = bank;
        }
    }

    /// The Superchip cart RAM as a writable slice for a state restore; empty on
    /// a board without one.
    pub(super) fn ram_mut(&mut self) -> &mut [u8] {
        match &mut self.ram {
            Some(ram) => ram.as_mut_slice(),
            None => &mut [],
        }
    }

    pub fn peek(&self, address: u16) -> u8 {
        let offset = (address & 0x0FFF) as usize;
        if let Some(ram) = &self.ram
            && offset < 2 * SUPERCHIP_RAM_SIZE
        {
            return ram[offset % SUPERCHIP_RAM_SIZE];
        }
        self.image[self.bank * BANK_SIZE + offset]
    }
}
