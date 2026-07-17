//! The SM83 `InstructionSet` front end must render exactly what the structural
//! decoder renders, and classify control flow the way step-over expects.

use missingno_core::isa::{Flow, InstructionSet};
use missingno_gb::Sm83;
use missingno_gb::cpu::instructions::{Instruction, instruction_length};

const ADDRESS: u32 = 0x0200;
const OP_LO: u8 = 0x34;
const OP_HI: u8 = 0x12;

fn reference_mnemonic(bytes: &[u8]) -> Option<String> {
    Instruction::decode(&mut bytes.iter().copied()).map(|i| i.to_string())
}

#[test]
fn tag_and_max_len() {
    assert_eq!(Sm83.id(), "sm83");
    assert_eq!(Sm83.max_len(), 3);
}

#[test]
fn mnemonic_and_length_match_the_structural_decoder() {
    for first in 0u16..=0xFF {
        let first = first as u8;
        let bytes = [first, OP_LO, OP_HI];
        let Some(reference) = reference_mnemonic(&bytes) else {
            continue;
        };
        let instruction = Sm83.decode(ADDRESS, &bytes);
        assert_eq!(
            instruction.mnemonic, reference,
            "mnemonic mismatch for opcode {first:#04x}"
        );
        assert_eq!(
            instruction.length as u16,
            instruction_length(first),
            "length mismatch for opcode {first:#04x}"
        );
    }
}

#[test]
fn cb_prefixed_second_bytes_match_the_structural_decoder() {
    for second in 0u16..=0xFF {
        let second = second as u8;
        let bytes = [0xCB, second, 0x00];
        let reference = reference_mnemonic(&bytes).expect("CB-prefixed opcodes always decode");
        let instruction = Sm83.decode(ADDRESS, &bytes);
        assert_eq!(
            instruction.mnemonic, reference,
            "mnemonic mismatch for CB {second:#04x}"
        );
        assert_eq!(
            instruction.length, 2,
            "CB-prefixed length for {second:#04x}"
        );
    }
}

fn flow_of(bytes: &[u8]) -> Flow {
    Sm83.decode(ADDRESS, bytes).flow
}

#[test]
fn call_is_a_call_to_the_operand() {
    assert!(matches!(
        flow_of(&[0xCD, OP_LO, OP_HI]),
        Flow::Call {
            target: Some(0x1234)
        }
    ));
}

#[test]
fn conditional_call_is_a_call() {
    // call nz, $1234
    assert!(matches!(
        flow_of(&[0xC4, OP_LO, OP_HI]),
        Flow::Call {
            target: Some(0x1234)
        }
    ));
}

#[test]
fn restart_is_a_call_to_the_vector() {
    assert!(matches!(
        flow_of(&[0xFF]),
        Flow::Call { target: Some(0x38) }
    ));
}

#[test]
fn unconditional_jump_targets_the_operand() {
    assert!(matches!(
        flow_of(&[0xC3, OP_LO, OP_HI]),
        Flow::Jump {
            target: Some(0x1234)
        }
    ));
}

#[test]
fn jump_through_hl_has_no_static_target() {
    assert!(matches!(flow_of(&[0xE9]), Flow::Jump { target: None }));
}

#[test]
fn conditional_jump_is_a_branch() {
    // jp nz, $1234
    assert!(matches!(
        flow_of(&[0xC2, OP_LO, OP_HI]),
        Flow::Branch {
            target: Some(0x1234)
        }
    ));
}

#[test]
fn relative_jump_targets_the_next_instruction_plus_offset() {
    // jr +5 from 0x0200: 0x0200 + 2 + 5 = 0x0207.
    assert!(matches!(
        flow_of(&[0x18, 0x05]),
        Flow::Jump {
            target: Some(0x0207)
        }
    ));
}

#[test]
fn conditional_relative_jump_is_a_branch_with_negative_offset() {
    // jr nz, -5 from 0x0200: 0x0200 + 2 - 5 = 0x01FD.
    assert!(matches!(
        flow_of(&[0x20, 0xFB]),
        Flow::Branch {
            target: Some(0x01FD)
        }
    ));
}

#[test]
fn returns_are_returns() {
    assert!(matches!(flow_of(&[0xC9]), Flow::Return)); // ret
    assert!(matches!(flow_of(&[0xD9]), Flow::Return)); // reti
    assert!(matches!(flow_of(&[0xC8]), Flow::Return)); // ret z
}
