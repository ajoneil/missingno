//! SM83 as a debugger instruction set: mnemonics and lengths come from the
//! structural decoder, flow is read off the decoded jump category.

use missingno_core::isa::{Flow, Instruction as IsaInstruction, InstructionSet};

use crate::cpu::instructions::jump::{Jump, Location};
use crate::cpu::instructions::{Address, Instruction, instruction_length};

/// The SM83 decode-for-display front end.
pub struct Sm83;

impl InstructionSet for Sm83 {
    fn id(&self) -> &'static str {
        "sm83"
    }

    fn max_len(&self) -> usize {
        3
    }

    fn decode(&self, address: u32, bytes: &[u8]) -> IsaInstruction {
        let opcode = bytes.first().copied().unwrap_or(0);
        let length = instruction_length(opcode) as u8;
        let decoded = Instruction::decode(&mut bytes.iter().copied());
        let (mnemonic, flow) = match decoded {
            Some(instruction) => {
                let flow = match &instruction {
                    Instruction::Jump(jump) => jump_flow(address as u16, jump),
                    _ => Flow::Sequential,
                };
                (instruction.to_string(), flow)
            }
            None => (format!("${opcode:02X}"), Flow::Sequential),
        };
        IsaInstruction {
            mnemonic,
            length,
            flow,
        }
    }
}

fn jump_flow(address: u16, jump: &Jump) -> Flow {
    match jump {
        Jump::Call(_, location) => Flow::Call {
            target: location_target(address, location),
        },
        Jump::Restart(vector) => Flow::Call {
            target: Some(*vector as u32),
        },
        Jump::Jump(None, location) => Flow::Jump {
            target: location_target(address, location),
        },
        Jump::Jump(Some(_), location) => Flow::Branch {
            target: location_target(address, location),
        },
        Jump::Return(_) | Jump::ReturnAndEnableInterrupts => Flow::Return,
    }
}

/// The static destination of a jump/call, or `None` when it is computed
/// (a jump through HL).
fn location_target(address: u16, location: &Location) -> Option<u32> {
    match location {
        Location::Address(Address::Fixed(target)) => Some(*target as u32),
        Location::Address(Address::Relative(offset)) => {
            Some(address.wrapping_add(2).wrapping_add(*offset as u16) as u32)
        }
        _ => None,
    }
}
