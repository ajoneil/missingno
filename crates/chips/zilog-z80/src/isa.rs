//! The Z80 as a debugger instruction set: mnemonics, lengths, and flow
//! come from the display disassembler.

use missingno_core::isa::{Instruction, InstructionSet, OperandClass};

use crate::disasm;

/// The Zilog Z80 decode-for-display front end.
pub struct Z80;

impl InstructionSet for Z80 {
    fn max_len(&self) -> usize {
        // DDCB d op, ld (ix+d),n, ld ix,nn, and the ED ld (nn),rp forms.
        4
    }

    fn decode(&self, address: u32, bytes: &[u8]) -> Instruction {
        let disassembly = disasm::disassemble(address as u16, bytes);
        Instruction {
            mnemonic: disassembly.mnemonic,
            length: disassembly.length,
            flow: disassembly.flow,
        }
    }

    fn classify_operand(&self, operand: &str) -> OperandClass {
        let operand = operand.trim();
        // `(hl)`, `(ix+$05)`, `($1234)` — and I/O ports `(c)`, `($10)`.
        if operand.starts_with('(') || operand.starts_with('[') {
            return OperandClass::Memory;
        }
        // "c" is both the carry register and the carry condition; like the
        // SM83, a condition always lands as the first operand, so a
        // standalone "c" reads as a condition. "p" and "m" are only ever
        // conditions — the Z80 has no registers by those names.
        if matches!(operand, "nz" | "z" | "nc" | "c" | "po" | "pe" | "p" | "m") {
            return OperandClass::Condition;
        }
        if matches!(
            operand,
            "a" | "b"
                | "d"
                | "e"
                | "h"
                | "l"
                | "i"
                | "r"
                | "af"
                | "af'"
                | "bc"
                | "de"
                | "hl"
                | "sp"
                | "ix"
                | "iy"
                | "ixh"
                | "ixl"
                | "iyh"
                | "iyl"
        ) {
            return OperandClass::Register;
        }
        if operand.starts_with('$') || operand.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return OperandClass::Immediate;
        }
        OperandClass::Plain
    }
}

/// The address a `call` at `pc` returns to; `None` when the instruction there
/// is not one. Both the unconditional and the condition-tested forms are three
/// bytes long.
pub fn step_over_target(opcode: u8, pc: u16) -> Option<u16> {
    let is_call = opcode == 0xCD || (opcode & 0xC7) == 0xC4;
    is_call.then(|| pc.wrapping_add(3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(operand: &str) -> OperandClass {
        Z80.classify_operand(operand)
    }

    #[test]
    fn steps_over_calls_only() {
        assert_eq!(step_over_target(0xCD, 0x1000), Some(0x1003));
        assert_eq!(step_over_target(0xC4, 0x1000), Some(0x1003));
        assert_eq!(step_over_target(0xFC, 0x1000), Some(0x1003));
        assert_eq!(step_over_target(0x00, 0x1000), None);
        assert_eq!(step_over_target(0xC7, 0x1000), None);
    }

    #[test]
    fn classifies_the_z80_lexicon() {
        assert_eq!(class("hl"), OperandClass::Register);
        assert_eq!(class("ix"), OperandClass::Register);
        assert_eq!(class("ixh"), OperandClass::Register);
        assert_eq!(class("af'"), OperandClass::Register);
        assert_eq!(class("r"), OperandClass::Register);
        assert_eq!(class("c"), OperandClass::Condition);
        assert_eq!(class("po"), OperandClass::Condition);
        assert_eq!(class("m"), OperandClass::Condition);
        assert_eq!(class("(hl)"), OperandClass::Memory);
        assert_eq!(class("(ix+$05)"), OperandClass::Memory);
        assert_eq!(class("(c)"), OperandClass::Memory);
        assert_eq!(class("$3f"), OperandClass::Immediate);
        assert_eq!(class("7"), OperandClass::Immediate);
    }
}
