//! The `InstructionSet` front end must render exactly what the disassembler
//! renders, and classify control flow the way step-over expects.

use missingno_core::isa::{Flow, InstructionSet};
use missingno_mos_6502::disasm;
use missingno_mos_6502::isa::Mos6502;

const ADDRESS: u32 = 0x0600;

/// Fixed operand bytes so mnemonics with operands compare deterministically.
const OP_LO: u8 = 0x34;
const OP_HI: u8 = 0x12;

fn decode(opcode: u8, address: u32) -> missingno_core::isa::Instruction {
    Mos6502.decode(address, &[opcode, OP_LO, OP_HI])
}

#[test]
fn max_len_covers_the_longest_opcode() {
    assert_eq!(Mos6502.max_len(), 3);
}

#[test]
fn mnemonic_and_length_match_the_disassembler() {
    for opcode in 0u16..=0xFF {
        let opcode = opcode as u8;
        let reference = disasm::disassemble(ADDRESS as u16, [opcode, OP_LO, OP_HI]);
        let instruction = decode(opcode, ADDRESS);
        assert_eq!(
            instruction.mnemonic, reference.mnemonic,
            "mnemonic mismatch for opcode {opcode:#04x}"
        );
        assert_eq!(
            instruction.length, reference.length,
            "length mismatch for opcode {opcode:#04x}"
        );
    }
}

#[test]
fn jsr_is_a_call_to_the_absolute_operand() {
    assert!(matches!(
        decode(0x20, ADDRESS).flow,
        Flow::Call {
            target: Some(0x1234)
        }
    ));
}

#[test]
fn absolute_jmp_targets_the_operand() {
    assert!(matches!(
        decode(0x4C, ADDRESS).flow,
        Flow::Jump {
            target: Some(0x1234)
        }
    ));
}

#[test]
fn indirect_jmp_has_no_static_target() {
    assert!(matches!(
        decode(0x6C, ADDRESS).flow,
        Flow::Jump { target: None }
    ));
}

#[test]
fn rts_and_rti_return() {
    assert!(matches!(decode(0x60, ADDRESS).flow, Flow::Return));
    assert!(matches!(decode(0x40, ADDRESS).flow, Flow::Return));
}

#[test]
fn brk_falls_through() {
    assert!(matches!(decode(0x00, ADDRESS).flow, Flow::Sequential));
}

#[test]
fn branch_target_is_relative_to_the_next_instruction() {
    // BEQ +0x34 from 0x0600: 0x0600 + 2 + 0x34 = 0x0636.
    let instruction = Mos6502.decode(ADDRESS, &[0xF0, 0x34, 0x00]);
    assert!(matches!(
        instruction.flow,
        Flow::Branch {
            target: Some(0x0636)
        }
    ));
}

#[test]
fn backward_branch_wraps_at_a_page_boundary() {
    // BEQ -0x10 from 0x0005: 0x0005 + 2 - 0x10 = 0xFFF7 (16-bit wrap).
    let instruction = Mos6502.decode(0x0005, &[0xF0, 0xF0, 0x00]);
    assert!(matches!(
        instruction.flow,
        Flow::Branch {
            target: Some(0xFFF7)
        }
    ));
}
