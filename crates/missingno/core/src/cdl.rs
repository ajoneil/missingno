//! Code/data logging: one flag byte per ROM byte, recording how each byte was
//! actually used while the debugger ran. The flag bits follow the Mesen/FCEUX
//! CDL convention so exported logs open elsewhere.
//!
//! How a CPU address reaches a ROM offset is the console's memory map, so
//! filling a log belongs with that console; what a filled log means does not.

pub const CODE: u8 = 0x01;
pub const DATA: u8 = 0x02;
/// missingno extension (a bit the Mesen GB set leaves unused): set on the
/// opcode byte only, so exact backward disassembly can anchor on real
/// instruction starts rather than operand bytes.
pub const INSTRUCTION_START: u8 = 0x04;
pub const JUMP_TARGET: u8 = 0x10;
pub const SUB_ENTRY_POINT: u8 = 0x80;

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

    pub fn flags_at(&self, address: u16) -> u8 {
        let offset = address.wrapping_sub(self.base) as usize;
        self.flags.get(offset).copied().unwrap_or(0)
    }

    /// A data byte that was never executed — the disassembly shows these as
    /// bytes instead of decoding garbage instructions through them.
    pub fn is_data(&self, address: u16) -> bool {
        let flags = self.flags_at(address);
        flags & DATA != 0 && flags & CODE == 0
    }

    pub fn is_instruction_start(&self, address: u16) -> bool {
        self.flags_at(address) & INSTRUCTION_START != 0
    }
}
