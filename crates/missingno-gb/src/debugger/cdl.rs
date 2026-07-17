//! Code/data logging: one flag byte per ROM byte, filled in while the
//! debugger runs, recording how each byte was actually used. The flag bits
//! follow the Mesen/FCEUX CDL convention so exported logs open elsewhere.
//!
//! Recording is instruction-grained: an interrupt that preempts a fetched
//! instruction can over-approximate by marking it executed one step early.

use std::path::Path;

pub use missingno_core::cdl::{
    CODE, CdlWindow, DATA, INSTRUCTION_START, JUMP_TARGET, SUB_ENTRY_POINT,
};

/// The size of the CPU-address window captured for the running view.
const WINDOW: usize = 512;

pub struct CodeDataLog {
    flags: Vec<u8>,
    dirty: bool,
}

impl CodeDataLog {
    pub fn new(rom_len: usize) -> Self {
        Self {
            flags: vec![0; rom_len],
            dirty: false,
        }
    }

    /// Restore a saved log; ignored (fresh log) when the size doesn't match
    /// the ROM — a stale log from a different ROM must not mislabel this one.
    pub fn from_bytes(bytes: Vec<u8>, rom_len: usize) -> Self {
        if bytes.len() == rom_len {
            Self {
                flags: bytes,
                dirty: false,
            }
        } else {
            Self::new(rom_len)
        }
    }

    /// The raw flag array — the interchange format (one byte per ROM byte).
    pub fn as_bytes(&self) -> &[u8] {
        &self.flags
    }

    pub fn load(path: &Path, rom_len: usize) -> Self {
        std::fs::read(path)
            .map(|bytes| Self::from_bytes(bytes, rom_len))
            .unwrap_or_else(|_| Self::new(rom_len))
    }

    /// Persist next to the ROM; skipped when nothing new was recorded.
    pub fn save(&self, path: &Path) {
        if self.dirty {
            let _ = std::fs::write(path, &self.flags);
        }
    }

    /// Map a CPU address to its flat ROM offset. Banked addresses need the
    /// mapped bank; anything outside ROM (or with an unknown bank) is None.
    fn flat(&self, address: u16, bank: Option<u16>) -> Option<usize> {
        let offset = match address {
            0x0000..=0x3fff => address as usize,
            0x4000..=0x7fff => bank? as usize * 0x4000 + (address as usize - 0x4000),
            _ => return None,
        };
        (offset < self.flags.len()).then_some(offset)
    }

    pub fn mark(&mut self, address: u16, bank: Option<u16>, bits: u8) {
        if let Some(offset) = self.flat(address, bank)
            && self.flags[offset] & bits != bits
        {
            self.flags[offset] |= bits;
            self.dirty = true;
        }
    }

    pub fn flags(&self, address: u16, bank: Option<u16>) -> u8 {
        self.flat(address, bank)
            .map(|offset| self.flags[offset])
            .unwrap_or(0)
    }

    /// ROM bytes with any flag set.
    pub fn coverage(&self) -> usize {
        self.flags.iter().filter(|&&flags| flags != 0).count()
    }

    /// A pre-resolved window of flags around `center`, for render paths that
    /// can't reach the log itself (the emu-thread snapshot).
    pub fn window(&self, center: u16, bank: Option<u16>) -> CdlWindow {
        let base = center.wrapping_sub((WINDOW / 4) as u16);
        CdlWindow::new(
            base,
            (0..WINDOW as u16)
                .map(|i| self.flags(base.wrapping_add(i), bank))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_map_through_banks_and_round_trip() {
        let mut cdl = CodeDataLog::new(0x10000);
        cdl.mark(0x0150, None, CODE);
        cdl.mark(0x4000, Some(3), CODE | JUMP_TARGET);
        cdl.mark(0x4000, Some(2), DATA);
        cdl.mark(0xc000, None, DATA); // WRAM — not ROM, ignored
        cdl.mark(0x4000, None, CODE); // unknown bank — ignored

        assert_eq!(cdl.flags(0x0150, None), CODE);
        assert_eq!(cdl.flags(0x4000, Some(3)), CODE | JUMP_TARGET);
        assert_eq!(cdl.flags(0x4000, Some(2)), DATA);
        assert_eq!(cdl.coverage(), 3);

        let restored = CodeDataLog::from_bytes(cdl.as_bytes().to_vec(), 0x10000);
        assert_eq!(restored.flags(0x4000, Some(3)), CODE | JUMP_TARGET);
        // A size mismatch discards the stale log.
        assert_eq!(
            CodeDataLog::from_bytes(cdl.as_bytes().to_vec(), 0x8000).coverage(),
            0
        );
    }

    #[test]
    fn window_resolves_flags_by_cpu_address() {
        let mut cdl = CodeDataLog::new(0x8000);
        cdl.mark(0x0150, None, CODE);
        cdl.mark(0x0151, None, DATA);

        let window = cdl.window(0x0150, None);
        assert_eq!(window.flags_at(0x0150), CODE);
        assert!(!window.is_data(0x0150));
        assert!(window.is_data(0x0151));
        assert_eq!(window.flags_at(0x2000), 0);
    }
}
