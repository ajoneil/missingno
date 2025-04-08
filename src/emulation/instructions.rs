use std::fmt::{self, Display, Formatter};

pub enum Instruction {
    NoOperation,

    Unknown(u8),
}

impl Instruction {
    pub fn decode(mut ops: impl Iterator<Item = u8>) -> Self {
        match ops.next().unwrap() {
            0x00 => Self::NoOperation,
            other => Self::Unknown(other),
        }
    }
}

impl Display for Instruction {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOperation => write!(f, "nop"),
            Self::Unknown(op) => write!(f, "Unknown op {:02x}", op),
        }
    }
}
