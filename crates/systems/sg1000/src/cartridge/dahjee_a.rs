//! The Taiwanese Type A memory expander: a pass-through unit between the
//! console and a ROM cartridge, carrying 8 KB the ported games run their work
//! in at $2000-$3FFF and a further kilobyte over the console's work-RAM window.
//! It holds `/DSRAM` high for the whole session, so the console's own kilobyte
//! is deselected and the expander's answers in its place.
//!
//! Provisional: no traced sheet of either expander exists — Enri marks the type
//! unconfirmed — so the decode is derived from the mechanism the edge offers, a
//! static `/DSRAM` drive over RAM wired to its own low address lines.

use std::ops::Range;

use super::WRAM_BASE;
use super::flat::Flat;

/// The window inside `/EXM2` the expander answers instead of the cartridge.
const EXPANSION: Range<u16> = 0x2000..0x4000;
const EXPANSION_SIZE: usize = 0x2000;
/// A0-A9 only, so the kilobyte repeats through the whole `/DSRAM` window.
const WORK_SIZE: usize = 0x400;

pub struct DahjeeA {
    rom: Flat,
    expansion: Vec<u8>,
    work: Vec<u8>,
}

/// The RAM cell an address selects, where the expander answers at all.
enum Cell {
    Expansion(usize),
    Work(usize),
}

impl DahjeeA {
    pub fn new(rom: &[u8]) -> DahjeeA {
        DahjeeA {
            rom: Flat::new(rom),
            expansion: vec![0; EXPANSION_SIZE],
            work: vec![0; WORK_SIZE],
        }
    }

    fn cell(address: u16) -> Option<Cell> {
        if EXPANSION.contains(&address) {
            Some(Cell::Expansion((address - EXPANSION.start) as usize))
        } else if address >= WRAM_BASE {
            Some(Cell::Work(address as usize & (WORK_SIZE - 1)))
        } else {
            None
        }
    }

    pub fn read(&self, address: u16) -> Option<u8> {
        match DahjeeA::cell(address) {
            Some(Cell::Expansion(cell)) => Some(self.expansion[cell]),
            Some(Cell::Work(cell)) => Some(self.work[cell]),
            None => self.rom.read(address),
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match DahjeeA::cell(address) {
            Some(Cell::Expansion(cell)) => self.expansion[cell] = value,
            Some(Cell::Work(cell)) => self.work[cell] = value,
            None => {}
        }
    }

    /// `/DSRAM` is held high for the whole session, not qualified by address.
    pub fn disables_console_ram(&self, _address: u16) -> bool {
        true
    }

    pub(super) fn ram(&self) -> Vec<&[u8]> {
        vec![&self.expansion, &self.work]
    }

    pub(super) fn ram_mut(&mut self) -> Vec<&mut [u8]> {
        vec![&mut self.expansion, &mut self.work]
    }
}
