use crate::emulation::instructions::{Source16, Target16};
use core::fmt;

pub enum StackPointer {
    Adjust(i8),
    Push(Source16),
    Pop(Target16),
}

impl fmt::Display for StackPointer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Adjust(offset) => write!(f, "add sp, {}", offset),
            Self::Pop(register) => write!(f, "pop {}", register),
            Self::Push(register) => write!(f, "push {}", register),
        }
    }
}

impl StackPointer {
    pub fn decode(op: u8, ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(match op {
            0xc1 => Self::Pop(Target16::bc()),
            0xd1 => Self::Pop(Target16::de()),
            0xe1 => Self::Pop(Target16::hl()),
            0xf1 => Self::Pop(Target16::af()),
            0xc5 => Self::Push(Source16::bc()),
            0xd5 => Self::Push(Source16::de()),
            0xe5 => Self::Push(Source16::hl()),
            0xf5 => Self::Push(Source16::af()),
            0xe8 => Self::Adjust(i8::from_le_bytes([ops.next()?])),
            _ => return None,
        })
    }
}
