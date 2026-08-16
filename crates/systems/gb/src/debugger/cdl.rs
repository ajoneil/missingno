//! Where the Game Boy's CPU addresses land in a code/data log: the bookkeeping
//! is generic, the memory map is not.
//!
//! Recording is instruction-grained: an interrupt that preempts a fetched
//! instruction can over-approximate by marking it executed one step early.

pub use missingno_core::cdl::{
    CODE, CdlWindow, CodeDataLog, DATA, INSTRUCTION_START, JUMP_TARGET, SUB_ENTRY_POINT,
};

/// The flat ROM offset a CPU address reads from. Banked addresses need the
/// mapped bank; anything outside ROM (or with an unknown bank) is None.
pub fn rom_offset(address: u16, bank: Option<u16>) -> Option<usize> {
    match address {
        0x0000..=0x3fff => Some(address as usize),
        0x4000..=0x7fff => Some(bank? as usize * 0x4000 + (address as usize - 0x4000)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_map_through_banks() {
        let mut cdl = CodeDataLog::new(0x10000);
        cdl.mark(rom_offset(0x0150, None), CODE);
        cdl.mark(rom_offset(0x4000, Some(3)), CODE | JUMP_TARGET);
        cdl.mark(rom_offset(0x4000, Some(2)), DATA);
        cdl.mark(rom_offset(0xc000, None), DATA); // WRAM — not ROM, ignored
        cdl.mark(rom_offset(0x4000, None), CODE); // unknown bank — ignored

        assert_eq!(cdl.flags(rom_offset(0x0150, None)), CODE);
        assert_eq!(cdl.flags(rom_offset(0x4000, Some(3))), CODE | JUMP_TARGET);
        assert_eq!(cdl.flags(rom_offset(0x4000, Some(2))), DATA);
        assert_eq!(cdl.coverage(), 3);
    }
}
