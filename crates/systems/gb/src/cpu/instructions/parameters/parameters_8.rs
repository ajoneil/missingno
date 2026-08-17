use core::fmt;

use crate::cpu::{Register8, instructions::Address};

/// An 8-bit location the CPU can both read and write.
#[derive(Copy, Clone, Debug)]
pub enum Place8 {
    Register(Register8),
    Memory(Address),
}

impl Place8 {
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

impl fmt::Display for Place8 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Register(register) => register.fmt(f),
            Self::Memory(address) => address.fmt(f),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Source8 {
    Place(Place8),
    Constant(u8),
}

impl From<Place8> for Source8 {
    fn from(place: Place8) -> Self {
        Self::Place(place)
    }
}

impl Source8 {
    pub fn constant(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Constant(ops.next()?))
    }

    pub fn a() -> Self {
        Place8::a().into()
    }

    pub fn b() -> Self {
        Place8::b().into()
    }

    pub fn c() -> Self {
        Place8::c().into()
    }

    pub fn d() -> Self {
        Place8::d().into()
    }

    pub fn e() -> Self {
        Place8::e().into()
    }

    pub fn h() -> Self {
        Place8::h().into()
    }

    pub fn l() -> Self {
        Place8::l().into()
    }

    pub fn address(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Place8::address(ops)?.into())
    }

    pub fn deref_bc() -> Self {
        Place8::deref_bc().into()
    }

    pub fn deref_de() -> Self {
        Place8::deref_de().into()
    }

    pub fn deref_hl() -> Self {
        Place8::deref_hl().into()
    }

    pub fn deref_hl_inc() -> Self {
        Place8::deref_hl_inc().into()
    }

    pub fn deref_hl_dec() -> Self {
        Place8::deref_hl_dec().into()
    }

    pub fn high(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Place8::high(ops)?.into())
    }

    pub fn high_c() -> Self {
        Place8::high_c().into()
    }
}

impl fmt::Display for Source8 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Place(place) => place.fmt(f),
            Self::Constant(value) => write!(f, "{}", value),
        }
    }
}
