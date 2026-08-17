//! Code/data logging: one flag byte per ROM byte, recording how each byte was
//! actually used while the debugger ran. The flag bits follow the Mesen/FCEUX
//! CDL convention so exported logs open elsewhere.
//!
//! How a CPU address reaches a ROM offset is the console's memory map, so
//! filling a log belongs with that console; what a filled log means does not.

use std::path::Path;

pub const CODE: u8 = 0x01;
pub const DATA: u8 = 0x02;
/// missingno extension (a bit the Mesen GB set leaves unused): set on the
/// opcode byte only, so exact backward disassembly can anchor on real
/// instruction starts rather than operand bytes.
pub const INSTRUCTION_START: u8 = 0x04;
pub const JUMP_TARGET: u8 = 0x10;
pub const SUB_ENTRY_POINT: u8 = 0x80;

/// The size of the CPU-address window captured for the running view.
const WINDOW: usize = 512;

/// One flag byte per ROM byte, grown while the debugger runs. Which offset a
/// CPU address reaches is the console's memory map, so every address-facing
/// call passes the offset that map resolved.
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

    pub fn mark(&mut self, offset: Option<usize>, bits: u8) {
        if let Some(offset) = self.in_rom(offset)
            && self.flags[offset] & bits != bits
        {
            self.flags[offset] |= bits;
            self.dirty = true;
        }
    }

    pub fn flags(&self, offset: Option<usize>) -> u8 {
        self.in_rom(offset)
            .map(|offset| self.flags[offset])
            .unwrap_or(0)
    }

    /// ROM bytes with any flag set.
    pub fn coverage(&self) -> usize {
        self.flags.iter().filter(|&&flags| flags != 0).count()
    }

    /// A pre-resolved window of flags around `center`, for render paths that
    /// can't reach the log itself (the emu-thread snapshot). `map` resolves a
    /// CPU address exactly as the marking calls did.
    pub fn window(&self, center: u16, map: impl Fn(u16) -> Option<usize>) -> CdlWindow {
        let base = center.wrapping_sub((WINDOW / 4) as u16);
        CdlWindow::new(
            base,
            (0..WINDOW as u16)
                .map(|i| self.flags(map(base.wrapping_add(i))))
                .collect(),
        )
    }

    fn in_rom(&self, offset: Option<usize>) -> Option<usize> {
        offset.filter(|&offset| offset < self.flags.len())
    }
}

/// A copied span of CDL flags by CPU address; zero flags outside the span.
#[derive(Clone, Default)]
pub struct CdlWindow {
    base: u16,
    flags: Vec<u8>,
}

impl CdlWindow {
    pub fn new(base: u16, flags: Vec<u8>) -> Self {
        CdlWindow { base, flags }
    }

    fn flags_at(&self, address: u16) -> u8 {
        let offset = address.wrapping_sub(self.base) as usize;
        self.flags.get(offset).copied().unwrap_or(0)
    }

    /// A data byte that was never executed — the disassembly shows these as
    /// bytes instead of decoding garbage instructions through them.
    pub(crate) fn is_data(&self, address: u16) -> bool {
        let flags = self.flags_at(address);
        flags & DATA != 0 && flags & CODE == 0
    }

    pub(crate) fn is_instruction_start(&self, address: u16) -> bool {
        self.flags_at(address) & INSTRUCTION_START != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An identity map: the console under test addresses its ROM flat.
    fn flat(address: u16) -> Option<usize> {
        Some(address as usize)
    }

    #[test]
    fn marks_accumulate_and_round_trip() {
        let mut log = CodeDataLog::new(0x2000);
        log.mark(flat(0x0100), CODE);
        log.mark(flat(0x0100), INSTRUCTION_START);
        log.mark(flat(0x0101), DATA);
        log.mark(None, CODE); // unmapped address
        log.mark(flat(0x4000), CODE); // past the end of the ROM

        assert_eq!(log.flags(flat(0x0100)), CODE | INSTRUCTION_START);
        assert_eq!(log.flags(flat(0x4000)), 0);
        assert_eq!(log.coverage(), 2);

        let restored = CodeDataLog::from_bytes(log.as_bytes().to_vec(), 0x2000);
        assert_eq!(restored.flags(flat(0x0100)), CODE | INSTRUCTION_START);
        // A size mismatch discards the stale log.
        assert_eq!(
            CodeDataLog::from_bytes(log.as_bytes().to_vec(), 0x1000).coverage(),
            0
        );
    }

    #[test]
    fn window_resolves_flags_by_cpu_address() {
        let mut log = CodeDataLog::new(0x2000);
        log.mark(flat(0x0150), CODE);
        log.mark(flat(0x0151), DATA);

        let window = log.window(0x0150, flat);
        assert_eq!(window.flags_at(0x0150), CODE);
        assert!(!window.is_data(0x0150));
        assert!(window.is_data(0x0151));
        assert_eq!(window.flags_at(0x1000), 0);
    }
}
