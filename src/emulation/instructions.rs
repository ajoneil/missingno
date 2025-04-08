use std::fmt::{self, Display, Formatter};

pub enum Instruction {
    NoOperation,
    Jump(JumpAddress),

    Unknown(u8),
}

pub enum JumpAddress {
    Absolute(u16),
}

impl Instruction {
    pub fn decode(mut ops: impl Iterator<Item = u8>) -> Self {
        match ops.next().unwrap() {
            0x00 => Self::NoOperation,

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
            Self::Jump(address) => match address {
                JumpAddress::Absolute(address) => write!(f, "jp {:04x}", address),
            },

            Self::Unknown(op) => write!(f, "Unknown op {:02x}", op),
        }
    }
}
