use crate::emulation::cpu::{Register8, Register16};
use std::fmt::{self, Display, Formatter};

pub enum Instruction {
    NoOperation,
    Jump(JumpAddress),

    Load16(Load16Target, Load16Source),

    XorA(Register8),

    Unknown(u8),
}

pub enum Load16Target {
    Register(Register16),
}

pub enum Load16Source {
    Constant(u16),
}

pub enum JumpAddress {
    Absolute(u16),
}

impl Instruction {
    pub fn decode(ops: &mut impl Iterator<Item = u8>) -> Self {
        match ops.next().unwrap() {
            0x00 => Self::NoOperation,

            0x01 => Self::Load16(
                Load16Target::Register(Register16::Bc),
                Load16Source::Constant(Self::read16(ops)),
            ),
            0x11 => Self::Load16(
                Load16Target::Register(Register16::De),
                Load16Source::Constant(Self::read16(ops)),
            ),
            0x21 => Self::Load16(
                Load16Target::Register(Register16::Hl),
                Load16Source::Constant(Self::read16(ops)),
            ),
            0x31 => Self::Load16(
                Load16Target::Register(Register16::Sp),
                Load16Source::Constant(Self::read16(ops)),
            ),

            0xaf => Self::XorA(Register8::A),
            0xa8 => Self::XorA(Register8::B),
            0xa9 => Self::XorA(Register8::C),
            0xaa => Self::XorA(Register8::D),
            0xab => Self::XorA(Register8::E),
            0xac => Self::XorA(Register8::H),
            0xad => Self::XorA(Register8::L),

            0xc3 => Self::Jump(JumpAddress::Absolute(Self::read16(ops))),

            other => Self::Unknown(other),
        }
    }

    fn read16(ops: &mut impl Iterator<Item = u8>) -> u16 {
        ops.next().unwrap() as u16 + ops.next().unwrap() as u16 * 0x100
    }
}

impl Display for Instruction {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOperation => write!(f, "nop"),

            Self::Load16(destination, source) => {
                write!(
                    f,
                    "ld {}, {}",
                    match destination {
                        Load16Target::Register(register) => register,
                    },
                    match source {
                        Load16Source::Constant(constant) => format!("{:04x}", constant),
                    }
                )
            }

            Self::XorA(register) => write!(f, "xor a, {}", register),

            Self::Jump(address) => match address {
                JumpAddress::Absolute(address) => write!(f, "jp {:04x}", address),
            },

            Self::Unknown(op) => write!(f, "Unknown op {:02x}", op),
        }
    }
}
