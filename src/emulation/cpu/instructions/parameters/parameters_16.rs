use super::Address;
use core::fmt;

pub enum Register16 {
    Bc,
    De,
    Hl,
    StackPointer,
    Af,
}

impl fmt::Display for Register16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Bc => "bc",
                Self::De => "de",
                Self::Hl => "hl",
                Self::StackPointer => "sp",
                Self::Af => "af",
            }
        )
    }
}

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

    pub fn af() -> Self {
        Self::Register(Register16::Af)
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

    pub fn sp_with_offset(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::StackPointerWithOffset(i8::from_le_bytes([
            ops.next()?
        ])))
    }

    pub fn af() -> Self {
        Self::Register(Register16::Af)
    }
}

impl fmt::Display for Source16 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source16::Constant(value) => value.fmt(f),
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
