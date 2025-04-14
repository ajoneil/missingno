use crate::emulator::cpu::registers::Register16;
use core::fmt;

pub enum Stack {
    Adjust(i8),
    Push(Register16),
    Pop(Register16),
}

impl fmt::Display for Stack {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Adjust(offset) => write!(f, "add sp, {}", offset),
            Self::Pop(register) => write!(f, "pop {}", register),
            Self::Push(register) => write!(f, "push {}", register),
        }
    }
}

impl Stack {
    pub fn decode(op: u8, ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(match op {
            0xc1 => Self::Pop(Register16::Bc),
            0xd1 => Self::Pop(Register16::De),
            0xe1 => Self::Pop(Register16::Hl),
            0xf1 => Self::Pop(Register16::Af),
            0xc5 => Self::Push(Register16::Bc),
            0xd5 => Self::Push(Register16::De),
            0xe5 => Self::Push(Register16::Hl),
            0xf5 => Self::Push(Register16::Af),
            0xe8 => Self::Adjust(i8::from_le_bytes([ops.next()?])),
            _ => return None,
        })
    }
}
