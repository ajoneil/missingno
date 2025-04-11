use super::Register16;
use core::fmt;

pub enum Address {
    Fixed(u16),
    Relative(i8),
    Hram(u8),
    HramPlusC,
    Dereference(Register16),
    DereferenceHlAndIncrement,
    DereferenceHlAndDecrement,
    DereferenceFixed(u16),
}

impl Address {
    pub fn fixed(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Fixed(u16::from_le_bytes([ops.next()?, ops.next()?])))
    }

    pub fn deref_fixed(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::DereferenceFixed(u16::from_le_bytes([
            ops.next()?,
            ops.next()?,
        ])))
    }

    pub fn relative(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Relative(i8::from_le_bytes([ops.next()?])))
    }

    pub fn hram(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Hram(ops.next()?))
    }

    pub fn deref_bc() -> Self {
        Self::Dereference(Register16::Bc)
    }

    pub fn deref_de() -> Self {
        Self::Dereference(Register16::De)
    }

    pub fn deref_hl() -> Self {
        Self::Dereference(Register16::Hl)
    }

    pub fn deref_hl_inc() -> Self {
        Self::DereferenceHlAndIncrement
    }

    pub fn deref_hl_dec() -> Self {
        Self::DereferenceHlAndDecrement
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fixed(address) => write!(f, "${:04x}", address),
            Self::Relative(offset) => write!(
                f,
                "($pc {} {})",
                if *offset >= 0 { "+" } else { "-" },
                offset.abs()
            ),
            Self::Hram(offset) => write!(f, "$hram[{}]", offset),
            Self::HramPlusC => write!(f, "$hram[c]"),
            Self::Dereference(register) => write!(f, "[${}]", register),
            Self::DereferenceHlAndIncrement => write!(f, "[$hl+]"),
            Self::DereferenceHlAndDecrement => write!(f, "[$hl-]"),
            Self::DereferenceFixed(address) => write!(f, "[${:04x}]", address),
        }
    }
}
