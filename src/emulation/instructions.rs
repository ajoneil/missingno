use crate::emulation::cpu::Register8;
use std::fmt::{self, Display, Formatter};

pub enum Instruction {
    NoOperation,
    Jump(JumpAddress),

    XorA(Register8),

    Unknown(u8),
}

pub enum JumpAddress {
    Absolute(u16),
}

impl Instruction {
    pub fn decode(mut ops: impl Iterator<Item = u8>) -> Self {
        match ops.next().unwrap() {
            0x00 => Self::NoOperation,

            0xaf => Self::XorA(Register8::A),
            0xa8 => Self::XorA(Register8::B),
            0xa9 => Self::XorA(Register8::C),
            0xaa => Self::XorA(Register8::D),
            0xab => Self::XorA(Register8::E),
            0xac => Self::XorA(Register8::H),
            0xad => Self::XorA(Register8::L),

            0xc3 => Self::Jump(JumpAddress::Absolute(
                ops.next().unwrap() as u16 + ops.next().unwrap() as u16 * 0x100,
            )),

            other => Self::Unknown(other),
        }
    }
}

impl Display for Instruction {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOperation => write!(f, "nop"),

            Self::XorA(register) => write!(f, "xor a, {}", register),

            Self::Jump(address) => match address {
                JumpAddress::Absolute(address) => write!(f, "jp {:04x}", address),
            },

            Self::Unknown(op) => write!(f, "Unknown op {:02x}", op),
        }
    }
}
