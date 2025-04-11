use core::fmt;

mod arithmetic;
mod bit_flag;
mod bit_shift;
mod bitwise;
mod carry_flag;
mod interrupt;
mod jump;
mod load;
mod stack_pointer;

use crate::emulation::cpu::{Register8, Register16};
use arithmetic::Arithmetic;
use bit_flag::BitFlag;
use bit_shift::BitShift;
use bitwise::Bitwise;
use carry_flag::CarryFlag;
use interrupt::Interrupt;
use jump::Jump;
use load::Load;
use stack_pointer::StackPointer;

pub enum Instruction {
    Load(Load),
    Arithmetic(Arithmetic),
    Bitwise(Bitwise),
    BitFlag(BitFlag),
    BitShift(BitShift),
    Jump(Jump),
    CarryFlag(CarryFlag),
    StackPointer(StackPointer),
    Interrupt(Interrupt),
    DecimalAdjustAccumulator,
    NoOperation,
    Stop,
    Invalid(u8),
}

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

    pub fn deref_fixed(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Memory(Address::deref_fixed(ops)?))
    }

    pub fn hram(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Memory(Address::hram(ops)?))
    }

    pub fn hram_c() -> Self {
        Self::Memory(Address::HramPlusC)
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

    pub fn deref_fixed(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Memory(Address::deref_fixed(ops)?))
    }

    pub fn hram(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Memory(Address::hram(ops)?))
    }

    pub fn hram_c() -> Self {
        Self::Memory(Address::HramPlusC)
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

impl Instruction {
    pub fn decode(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        let op = ops.next()?;

        Some(match op {
            0x00 => Self::NoOperation,
            0x10 => Self::Stop,
            0x07 => Self::BitShift(BitShift::RotateA(
                bit_shift::Direction::Left,
                bit_shift::Carry::SetOnly,
            )),
            0x17 => Self::BitShift(BitShift::RotateA(
                bit_shift::Direction::Left,
                bit_shift::Carry::Through,
            )),
            0x27 => Self::DecimalAdjustAccumulator,
            0x37 => Self::CarryFlag(CarryFlag::Set),
            0x0f => Self::BitShift(BitShift::RotateA(
                bit_shift::Direction::Right,
                bit_shift::Carry::SetOnly,
            )),
            0x1f => Self::BitShift(BitShift::RotateA(
                bit_shift::Direction::Right,
                bit_shift::Carry::Through,
            )),
            0x3f => Self::CarryFlag(CarryFlag::Complement),
            0x76 => Self::Interrupt(Interrupt::Await),
            0xf3 => Self::Interrupt(Interrupt::Disable),
            0xfb => Self::Interrupt(Interrupt::Enable),
            0xcb => {
                let op = ops.next()?;
                match op {
                    0x00..=0x3f => Self::BitFlag(BitFlag::decode(op)),
                    _ => Self::BitShift(BitShift::decode(op)),
                }
            }
            _ => {
                if let Some(load) = Load::decode(op, ops) {
                    Self::Load(load)
                } else if let Some(arithmetic) = Arithmetic::decode(op, ops) {
                    Self::Arithmetic(arithmetic)
                } else if let Some(bitwise) = Bitwise::decode(op, ops) {
                    Self::Bitwise(bitwise)
                } else if let Some(jump) = Jump::decode(op, ops) {
                    Self::Jump(jump)
                } else if let Some(stack_pointer) = StackPointer::decode(op, ops) {
                    Self::StackPointer(stack_pointer)
                } else {
                    Self::Invalid(op)
                }
            }
        })
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(load) => load.fmt(f),
            Self::Arithmetic(arithmetic) => arithmetic.fmt(f),
            Self::Bitwise(bitwise) => bitwise.fmt(f),
            Self::BitFlag(bit_flag) => bit_flag.fmt(f),
            Self::BitShift(bit_shift) => bit_shift.fmt(f),
            Self::Jump(jump) => jump.fmt(f),
            Self::CarryFlag(carry_flag) => carry_flag.fmt(f),
            Self::StackPointer(stack_pointer) => stack_pointer.fmt(f),
            Self::Interrupt(interrupt) => interrupt.fmt(f),
            Self::DecimalAdjustAccumulator => write!(f, "daa"),
            Self::NoOperation => write!(f, "nop"),
            Self::Stop => write!(f, "stop"),
            Self::Invalid(op) => write!(f, "Invalid op {:02x}", op),
        }
    }
}
