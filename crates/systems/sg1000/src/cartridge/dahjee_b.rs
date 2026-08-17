//! The Taiwanese Type B memory expander: a pass-through unit carrying 8 KB over
//! the console's work-RAM window, with `/DSRAM` held high so the console's own
//! kilobyte is deselected. A0-A12 only, so the 8 KB answers $C000-$FFFF twice.
//! The cartridge's ROM runs flat through both cartridge windows beneath it.
//!
//! Provisional: no traced sheet of either expander exists — Enri marks the type
//! unconfirmed, and puts the RAM at $C000-$DFFF where MAME mirrors it to the top
//! of the map — so the decode is derived from the mechanism the edge offers, a
//! static `/DSRAM` drive over RAM wired to its own low address lines.

use super::WRAM_BASE;
use super::flat::Flat;

const RAM_SIZE: usize = 0x2000;

pub struct DahjeeB {
    rom: Flat,
    ram: Vec<u8>,
}

impl DahjeeB {
    pub fn new(rom: &[u8]) -> DahjeeB {
        DahjeeB {
            rom: Flat::new(rom),
            ram: vec![0; RAM_SIZE],
        }
    }

    fn cell(address: u16) -> Option<usize> {
        (address >= WRAM_BASE).then_some(address as usize & (RAM_SIZE - 1))
    }

    pub fn read(&self, address: u16) -> Option<u8> {
        match DahjeeB::cell(address) {
            Some(cell) => Some(self.ram[cell]),
            None => self.rom.read(address),
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        if let Some(cell) = DahjeeB::cell(address) {
            self.ram[cell] = value;
        }
    }

    /// `/DSRAM` is held high for the whole session, not qualified by address.
    pub fn disables_console_ram(&self, _address: u16) -> bool {
        true
    }

    pub(super) fn ram(&self) -> &[u8] {
        &self.ram
    }

    pub(super) fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram
    }
}
