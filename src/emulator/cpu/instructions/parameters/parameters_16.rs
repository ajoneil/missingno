use core::fmt;

use crate::emulator::cpu::{Register16, instructions::Address};

#[derive(Clone)]
pub enum Target16 {
    Register(Register16),
    Memory(Address),
}

impl Target16 {
    pub fn bc() -> Self {
        Self::Register(Register16::Bc)
    }

    pub fn de() -> Self {
        Self::Register(Register16::De)
    }

    pub fn hl() -> Self {
        Self::Register(Register16::Hl)
    }

    pub fn sp() -> Self {
        Self::Register(Register16::StackPointer)
    }

    pub fn memory(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Memory(Address::fixed(ops)?))
    }
}

impl fmt::Display for Target16 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Target16::Register(register) => register.to_string(),
                Target16::Memory(address) => address.to_string(),
            }
        )
    }
}

#[derive(Clone)]
pub enum Source16 {
    Constant(u16),
    Register(Register16),
    StackPointerWithOffset(i8),
}

impl Source16 {
    pub fn constant(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Constant(u16::from_le_bytes([
            ops.next()?,
            ops.next()?,
        ])))
    }

    pub fn hl() -> Self {
        Self::Register(Register16::Hl)
    }

    pub fn sp() -> Self {
        Self::Register(Register16::StackPointer)
    }

    pub fn sp_with_offset(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::StackPointerWithOffset(i8::from_le_bytes([
            ops.next()?
        ])))
    }
}

impl fmt::Display for Source16 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source16::Constant(value) => write!(f, "${:04x}", value),
            Source16::Register(register) => register.fmt(f),
            Source16::StackPointerWithOffset(offset) => write!(
                f,
                "sp {} {}",
                if *offset >= 0 { "+" } else { "-" },
                offset.abs()
            ),
        }
    }
}
