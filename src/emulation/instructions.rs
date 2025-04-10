use crate::emulation::cpu::{Pointer, Register8, Register16};
use std::fmt::{self, Display, Formatter};

pub enum Instruction {
    NoOperation,
    Jump(JumpAddress),

    Load8(Load8Target, Load8Source),
    Load16(Load16Target, Load16Source),

    Decrement8(Register8),

    XorA(Register8),

    Unknown(u8),
}

pub enum JumpAddress {
    Absolute(u16),
}

pub enum Load8Target {
    Register(Register8),
    Pointer(Pointer),
}

pub enum Load8Source {
    Constant(u8),
    Register(Register8),
}

pub enum Load16Target {
    Register(Register16),
}

pub enum Load16Source {
    Constant(u16),
}

impl Instruction {
    pub fn decode(ops: &mut impl Iterator<Item = u8>) -> Self {
        match ops.next().unwrap() {
            0x00 => Self::NoOperation,

            0xc3 => Self::Jump(JumpAddress::Absolute(Self::read16(ops))),

            0x3e => Self::Load8(
                Load8Target::Register(Register8::A),
                Load8Source::Constant(ops.next().unwrap()),
            ),
            0x06 => Self::Load8(
                Load8Target::Register(Register8::B),
                Load8Source::Constant(ops.next().unwrap()),
            ),
            0x0e => Self::Load8(
                Load8Target::Register(Register8::C),
                Load8Source::Constant(ops.next().unwrap()),
            ),
            0x16 => Self::Load8(
                Load8Target::Register(Register8::D),
                Load8Source::Constant(ops.next().unwrap()),
            ),
            0x1e => Self::Load8(
                Load8Target::Register(Register8::E),
                Load8Source::Constant(ops.next().unwrap()),
            ),
            0x26 => Self::Load8(
                Load8Target::Register(Register8::H),
                Load8Source::Constant(ops.next().unwrap()),
            ),
            0x2e => Self::Load8(
                Load8Target::Register(Register8::L),
                Load8Source::Constant(ops.next().unwrap()),
            ),

            0x22 => Self::Load8(
                Load8Target::Pointer(Pointer::HlIncrement),
                Load8Source::Register(Register8::A),
            ),
            0x32 => Self::Load8(
                Load8Target::Pointer(Pointer::HlDecrement),
                Load8Source::Register(Register8::A),
            ),

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
                Load16Target::Register(Register16::StackPointer),
                Load16Source::Constant(Self::read16(ops)),
            ),

            0x3d => Self::Decrement8(Register8::A),
            0x05 => Self::Decrement8(Register8::B),
            0x0d => Self::Decrement8(Register8::C),
            0x15 => Self::Decrement8(Register8::D),
            0x1d => Self::Decrement8(Register8::E),
            0x25 => Self::Decrement8(Register8::H),
            0x2d => Self::Decrement8(Register8::L),

            0xaf => Self::XorA(Register8::A),
            0xa8 => Self::XorA(Register8::B),
            0xa9 => Self::XorA(Register8::C),
            0xaa => Self::XorA(Register8::D),
            0xab => Self::XorA(Register8::E),
            0xac => Self::XorA(Register8::H),
            0xad => Self::XorA(Register8::L),

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

            Self::Jump(address) => match address {
                JumpAddress::Absolute(address) => write!(f, "jp {:04x}", address),
            },

            Self::Decrement8(register) => write!(f, "dec {}", register),

            Self::Load8(destination, source) => {
                write!(
                    f,
                    "ld {}, {}",
                    match destination {
                        Load8Target::Register(register) => register.to_string(),
                        Load8Target::Pointer(pointer) => pointer.to_string(),
                    },
                    match source {
                        Load8Source::Constant(value) => format!("{:02x}", value),
                        Load8Source::Register(register) => register.to_string(),
                    }
                )
            }

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

            Self::Unknown(op) => write!(f, "Unknown op {:02x}", op),
        }
    }
}
