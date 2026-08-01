//! Fixtures shared by the debugger's test modules: bank-stamped images and a
//! debugger over one.

use crate::TvStandard;
use crate::cartridge::{CartType, DumpFit};
use crate::console::Vcs;
use crate::debugger::Debugger;

/// Each 4 KB window-sized chunk filled with its bank index, so a linear read
/// reveals which bank a byte came from.
pub(super) fn bank_stamped(size: usize) -> Vec<u8> {
    let mut rom = vec![0u8; size];
    for (i, bank) in rom.chunks_mut(0x1000).enumerate() {
        bank.fill(i as u8);
    }
    rom
}

pub(super) fn debugger(rom: &[u8], cart_type: CartType) -> Debugger {
    Debugger::new(
        Vcs::new(rom, TvStandard::Ntsc, Some(cart_type), DumpFit::Exact).expect("valid image"),
    )
}

/// Write a 6507 reset vector pointing at `$F000` into a 4 KB bank image.
pub(super) fn reset_to_f000(bank: &mut [u8]) {
    bank[0xFFC] = 0x00;
    bank[0xFFD] = 0xF0;
}
