//! The two Sega boards that carry work RAM beside the ROM: Othello's 2 KB
//! (171-5044) and The Castle's 8 KB (171-5382). Both take the RAM's chip select
//! from `/EXM1` alone, with `/RD` and `/WR` on its output and write enables and
//! the window's upper address lines left unconnected — so the RAM answers all
//! of $8000-$BFFF and repeats through it, eight times on Othello and twice on
//! The Castle. Neither board brings `/DSRAM` onto the sheet, so the console's
//! own work RAM stays visible.

use super::flat::Flat;
use super::{EXM1_BASE, WRAM_BASE};

/// Othello's D4016C: A0-A10.
pub const OTHELLO_RAM: usize = 0x800;
/// The Castle's 6264-style SRAM: A0-A12, with CE2 tied high.
pub const CASTLE_RAM: usize = 0x2000;

pub struct SegaRam {
    rom: Flat,
    ram: Vec<u8>,
}

impl SegaRam {
    pub fn new(rom: &[u8], ram_size: usize) -> SegaRam {
        SegaRam {
            rom: Flat::new(rom),
            ram: vec![0; ram_size],
        }
    }

    /// The cell `/EXM1` selects, the RAM's own address lines deciding which.
    fn cell(&self, address: u16) -> Option<usize> {
        (EXM1_BASE..WRAM_BASE)
            .contains(&address)
            .then(|| address as usize & (self.ram.len() - 1))
    }

    pub fn read(&self, address: u16) -> Option<u8> {
        match self.cell(address) {
            Some(cell) => Some(self.ram[cell]),
            None => self.rom.read(address),
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        if let Some(cell) = self.cell(address) {
            self.ram[cell] = value;
        }
    }

    pub(super) fn ram(&self) -> &[u8] {
        &self.ram
    }

    pub(super) fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram
    }
}
