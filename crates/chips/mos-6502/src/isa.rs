//! The 6502 as a debugger instruction set: mnemonics and lengths come from the
//! disassembler, flow is classified from the decode table.

use missingno_core::isa::{Flow, Instruction, InstructionSet, OperandClass};

use crate::decode::{DECODE, Mode, Op};
use crate::disasm;

/// The NMOS 6502 decode-for-display front end.
pub struct Mos6502;

impl InstructionSet for Mos6502 {
    fn max_len(&self) -> usize {
        3
    }

    fn decode(&self, address: u32, bytes: &[u8]) -> Instruction {
        let mut operands = [0u8; 3];
        for (slot, byte) in operands.iter_mut().zip(bytes) {
            *slot = *byte;
        }
        let disassembly = disasm::disassemble(address as u16, operands);
        Instruction {
            mnemonic: disassembly.mnemonic,
            length: disassembly.length,
            flow: flow(address as u16, operands),
        }
    }

    fn classify_operand(&self, operand: &str) -> OperandClass {
        let operand = operand.trim();
        // `#$44` is a literal; `($44,x)` and friends are indirect memory; a bare
        // `$1234` is a direct address, unlike the SM83 where `$` reads immediate.
        if operand.starts_with('#') {
            OperandClass::Immediate
        } else if operand.starts_with('(') {
            OperandClass::Memory
        } else if matches!(operand, "a" | "x" | "y") {
            OperandClass::Register
        } else if operand.starts_with('$') {
            OperandClass::Memory
        } else {
            OperandClass::Plain
        }
    }
}

/// Control-flow class, deliberately conservative: only the transfers today's
/// step-over and disassembler recognise. Everything else — BRK included — is
/// treated as fall-through.
fn flow(address: u16, bytes: [u8; 3]) -> Flow {
    let instr = DECODE[bytes[0] as usize];
    let word = u16::from_le_bytes([bytes[1], bytes[2]]) as u32;
    match (instr.op, instr.mode) {
        (Op::Jsr, _) => Flow::Call { target: Some(word) },
        (Op::Jmp, Mode::Absolute) => Flow::Jump { target: Some(word) },
        (Op::Jmp, Mode::Indirect) => Flow::Jump { target: None },
        (Op::Branch(..), _) => {
            let target = address.wrapping_add(2).wrapping_add(bytes[1] as i8 as u16);
            Flow::Branch {
                target: Some(target as u32),
            }
        }
        (Op::Rts, _) | (Op::Rti, _) => Flow::Return,
        _ => Flow::Sequential,
    }
}

/// The address a `jsr` at `pc` returns to; `None` when the instruction there is
/// not one.
pub fn step_over_target(opcode: u8, pc: u16) -> Option<u16> {
    matches!(DECODE[opcode as usize].op, Op::Jsr).then(|| pc.wrapping_add(3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(operand: &str) -> OperandClass {
        Mos6502.classify_operand(operand)
    }

    #[test]
    fn steps_over_subroutine_calls_only() {
        assert_eq!(step_over_target(0x20, 0x1000), Some(0x1003));
        assert_eq!(step_over_target(0xEA, 0x1000), None);
        assert_eq!(step_over_target(0x4C, 0x1000), None);
    }

    #[test]
    fn classifies_the_6502_lexicon() {
        assert_eq!(class("a"), OperandClass::Register);
        assert_eq!(class("x"), OperandClass::Register);
        assert_eq!(class("y"), OperandClass::Register);
        assert_eq!(class("#$44"), OperandClass::Immediate);
        assert_eq!(class("$44"), OperandClass::Memory);
        assert_eq!(class("$1234,x"), OperandClass::Memory);
        assert_eq!(class("($44,x)"), OperandClass::Memory);
        assert_eq!(class("($1234)"), OperandClass::Memory);
    }
}
