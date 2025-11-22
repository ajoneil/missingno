use core::fmt;

use crate::game_boy::cpu::{Register8, instructions::Address};

#[derive(Clone)]
pub enum Target8 {
    Register(Register8),
    Memory(Address),
}

impl Target8 {
    pub fn a() -> Self {
        Self::Register(Register8::A)
    }

    pub fn b() -> Self {
        Self::Register(Register8::B)
    }

    pub fn c() -> Self {
        Self::Register(Register8::C)
    }

    pub fn d() -> Self {
        Self::Register(Register8::D)
    }

    pub fn e() -> Self {
        Self::Register(Register8::E)
    }

    pub fn h() -> Self {
        Self::Register(Register8::H)
    }

    pub fn l() -> Self {
        Self::Register(Register8::L)
    }

    pub fn address(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Memory(Address::Fixed(u16::from_le_bytes([
            ops.next()?,
            ops.next()?,
        ]))))
    }

    pub fn deref_bc() -> Self {
        Self::Memory(Address::deref_bc())
    }

    pub fn deref_de() -> Self {
        Self::Memory(Address::deref_de())
    }

    pub fn deref_hl() -> Self {
        Self::Memory(Address::deref_hl())
    }

    pub fn deref_hl_inc() -> Self {
        Self::Memory(Address::deref_hl_inc())
    }

    pub fn deref_hl_dec() -> Self {
        Self::Memory(Address::deref_hl_dec())
    }

    pub fn high(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Memory(Address::high(ops)?))
    }

    pub fn high_c() -> Self {
        Self::Memory(Address::HighPlusC)
    }

    pub fn to_source(&self) -> Source8 {
        match self {
            Self::Register(register) => Source8::Register(*register),
            Self::Memory(address) => Source8::Memory(*address),
        }
    }
}

impl fmt::Display for Target8 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Register(register) => register.to_string(),
                Self::Memory(address) => address.to_string(),
            }
        )
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Source8 {
    Constant(u8),
    Register(Register8),
    Memory(Address),
}

impl Source8 {
    pub fn constant(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Constant(ops.next()?))
    }

    pub fn a() -> Self {
        Self::Register(Register8::A)
    }

    pub fn b() -> Self {
        Self::Register(Register8::B)
    }

    pub fn c() -> Self {
        Self::Register(Register8::C)
    }

    pub fn d() -> Self {
        Self::Register(Register8::D)
    }

    pub fn e() -> Self {
        Self::Register(Register8::E)
    }

    pub fn h() -> Self {
        Self::Register(Register8::H)
    }

    pub fn l() -> Self {
        Self::Register(Register8::L)
    }

    pub fn address(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Memory(Address::Fixed(u16::from_le_bytes([
            ops.next()?,
            ops.next()?,
        ]))))
    }

    pub fn deref_bc() -> Self {
        Self::Memory(Address::deref_bc())
    }

    pub fn deref_de() -> Self {
        Self::Memory(Address::deref_de())
    }

    pub fn deref_hl() -> Self {
        Self::Memory(Address::deref_hl())
    }

    pub fn deref_hl_inc() -> Self {
        Self::Memory(Address::deref_hl_inc())
    }

    pub fn deref_hl_dec() -> Self {
        Self::Memory(Address::deref_hl_dec())
    }

    pub fn high(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Memory(Address::high(ops)?))
    }

    pub fn high_c() -> Self {
        Self::Memory(Address::HighPlusC)
    }
}

impl fmt::Display for Source8 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Source8::Constant(value) => format!("{}", value),
                Source8::Register(register) => register.to_string(),
                Source8::Memory(address) => address.to_string(),
            }
        )
    }
}
