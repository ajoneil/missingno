use core::fmt;

use crate::game_boy::cpu::{
    Register16,
    instructions::{Source8, Target8},
};

#[derive(Clone)]
pub enum Arithmetic {
    Arithmetic8(Arithmetic8),
    Arithmetic16(Arithmetic16),
}

#[derive(Clone)]
pub enum Arithmetic8 {
    Increment(Target8),
    Decrement(Target8),
    AddA(Source8),
    SubtractA(Source8),
    AddACarry(Source8),
    SubtractACarry(Source8),
    CompareA(Source8),
}

impl Into<Arithmetic> for Arithmetic8 {
    fn into(self) -> Arithmetic {
        Arithmetic::Arithmetic8(self)
    }
}

impl Arithmetic8 {
    pub fn inc(target: Target8) -> Arithmetic {
        Self::Increment(target).into()
    }

    pub fn dec(target: Target8) -> Arithmetic {
        Self::Decrement(target).into()
    }

    pub fn add_a(source: Source8) -> Arithmetic {
        Self::AddA(source).into()
    }

    pub fn sub_a(source: Source8) -> Arithmetic {
        Self::SubtractA(source).into()
    }

    pub fn adc_a(source: Source8) -> Arithmetic {
        Self::AddACarry(source).into()
    }

    pub fn sbc_a(source: Source8) -> Arithmetic {
        Self::SubtractACarry(source).into()
    }

    pub fn cp_a(source: Source8) -> Arithmetic {
        Self::CompareA(source).into()
    }
}

#[derive(Clone)]
pub enum Arithmetic16 {
    Increment(Register16),
    Decrement(Register16),
    AddHl(Register16),
}

impl Into<Arithmetic> for Arithmetic16 {
    fn into(self) -> Arithmetic {
        Arithmetic::Arithmetic16(self)
    }
}

impl Arithmetic16 {
    pub fn inc(register: Register16) -> Arithmetic {
        Self::Increment(register).into()
    }

    pub fn dec(register: Register16) -> Arithmetic {
        Self::Decrement(register).into()
    }

    pub fn add_hl(register: Register16) -> Arithmetic {
        Self::AddHl(register).into()
    }
}

impl Arithmetic {
    pub fn decode(op: u8, ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(match op {
            0x04 => Arithmetic8::inc(Target8::b()),
            0x14 => Arithmetic8::inc(Target8::d()),
            0x24 => Arithmetic8::inc(Target8::h()),
            0x34 => Arithmetic8::inc(Target8::deref_hl()),
            0x05 => Arithmetic8::dec(Target8::b()),
            0x15 => Arithmetic8::dec(Target8::d()),
            0x25 => Arithmetic8::dec(Target8::h()),
            0x35 => Arithmetic8::dec(Target8::deref_hl()),
            0x0c => Arithmetic8::inc(Target8::c()),
            0x1c => Arithmetic8::inc(Target8::e()),
            0x2c => Arithmetic8::inc(Target8::l()),
            0x3c => Arithmetic8::inc(Target8::a()),
            0x0d => Arithmetic8::dec(Target8::c()),
            0x1d => Arithmetic8::dec(Target8::e()),
            0x2d => Arithmetic8::dec(Target8::l()),
            0x3d => Arithmetic8::dec(Target8::a()),
            0x80 => Arithmetic8::add_a(Source8::b()),
            0x90 => Arithmetic8::sub_a(Source8::b()),
            0x81 => Arithmetic8::add_a(Source8::c()),
            0x91 => Arithmetic8::sub_a(Source8::c()),
            0x82 => Arithmetic8::add_a(Source8::d()),
            0x92 => Arithmetic8::sub_a(Source8::d()),
            0x83 => Arithmetic8::add_a(Source8::e()),
            0x93 => Arithmetic8::sub_a(Source8::e()),
            0x84 => Arithmetic8::add_a(Source8::h()),
            0x94 => Arithmetic8::sub_a(Source8::h()),
            0x85 => Arithmetic8::add_a(Source8::l()),
            0x95 => Arithmetic8::sub_a(Source8::l()),
            0x86 => Arithmetic8::add_a(Source8::deref_hl()),
            0x96 => Arithmetic8::sub_a(Source8::deref_hl()),
            0x87 => Arithmetic8::add_a(Source8::a()),
            0x97 => Arithmetic8::sub_a(Source8::a()),
            0x88 => Arithmetic8::adc_a(Source8::b()),
            0x98 => Arithmetic8::sbc_a(Source8::b()),
            0xb8 => Arithmetic8::cp_a(Source8::b()),
            0x89 => Arithmetic8::adc_a(Source8::c()),
            0x99 => Arithmetic8::sbc_a(Source8::c()),
            0xb9 => Arithmetic8::cp_a(Source8::c()),
            0x8a => Arithmetic8::adc_a(Source8::d()),
            0x9a => Arithmetic8::sbc_a(Source8::d()),
            0xba => Arithmetic8::cp_a(Source8::d()),
            0x8b => Arithmetic8::adc_a(Source8::e()),
            0x9b => Arithmetic8::sbc_a(Source8::e()),
            0xbb => Arithmetic8::cp_a(Source8::e()),
            0x8c => Arithmetic8::adc_a(Source8::h()),
            0x9c => Arithmetic8::sbc_a(Source8::h()),
            0xbc => Arithmetic8::cp_a(Source8::h()),
            0x8d => Arithmetic8::adc_a(Source8::l()),
            0x9d => Arithmetic8::sbc_a(Source8::l()),
            0xbd => Arithmetic8::cp_a(Source8::l()),
            0x8e => Arithmetic8::adc_a(Source8::deref_hl()),
            0x9e => Arithmetic8::sbc_a(Source8::deref_hl()),
            0xbe => Arithmetic8::cp_a(Source8::deref_hl()),
            0x8f => Arithmetic8::adc_a(Source8::a()),
            0x9f => Arithmetic8::sbc_a(Source8::a()),
            0xbf => Arithmetic8::cp_a(Source8::a()),
            0xc6 => Arithmetic8::add_a(Source8::constant(ops)?),
            0xd6 => Arithmetic8::sub_a(Source8::constant(ops)?),
            0xce => Arithmetic8::add_a(Source8::constant(ops)?),
            0xde => Arithmetic8::sbc_a(Source8::constant(ops)?),
            0xfe => Arithmetic8::cp_a(Source8::constant(ops)?),

            0x03 => Arithmetic16::inc(Register16::Bc),
            0x13 => Arithmetic16::inc(Register16::De),
            0x23 => Arithmetic16::inc(Register16::Hl),
            0x33 => Arithmetic16::inc(Register16::StackPointer),
            0x09 => Arithmetic16::add_hl(Register16::Bc),
            0x19 => Arithmetic16::add_hl(Register16::De),
            0x29 => Arithmetic16::add_hl(Register16::Hl),
            0x39 => Arithmetic16::add_hl(Register16::StackPointer),
            0x0b => Arithmetic16::dec(Register16::Bc),
            0x1b => Arithmetic16::dec(Register16::De),
            0x2b => Arithmetic16::dec(Register16::Hl),
            0x3b => Arithmetic16::dec(Register16::StackPointer),

            _ => return None,
        })
    }
}

impl fmt::Display for Arithmetic {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Arithmetic::Arithmetic8(arithmetic) => match arithmetic {
                Arithmetic8::Increment(target) => write!(f, "inc {}", target),
                Arithmetic8::Decrement(target) => write!(f, "dec {}", target),
                Arithmetic8::AddA(source) => write!(f, "add a, {}", source),
                Arithmetic8::SubtractA(source) => write!(f, "sub a, {}", source),
                Arithmetic8::AddACarry(source) => write!(f, "adc a, {}", source),
                Arithmetic8::SubtractACarry(source) => write!(f, "sbc a, {}", source),
                Arithmetic8::CompareA(source) => write!(f, "cp a, {}", source),
            },
            Arithmetic::Arithmetic16(arithmetic) => match arithmetic {
                Arithmetic16::Increment(target) => write!(f, "inc {}", target),
                Arithmetic16::Decrement(target) => write!(f, "dec {}", target),
                Arithmetic16::AddHl(target) => write!(f, "add hl, {}", target),
            },
        }
    }
}
