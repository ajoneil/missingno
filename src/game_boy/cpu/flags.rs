use core::fmt;

use bitflags::bitflags;
use nanoserde::{DeRon, DeRonErr, DeRonState, SerRon, SerRonState};

#[derive(Clone)]
#[allow(dead_code)]
pub enum Flag {
    Zero,
    Negative,
    HalfCarry,
    Carry,
}

impl From<Flag> for Flags {
    fn from(flag: Flag) -> Self {
        match flag {
            Flag::Zero => Flags::ZERO,
            Flag::Negative => Flags::NEGATIVE,
            Flag::HalfCarry => Flags::HALF_CARRY,
            Flag::Carry => Flags::CARRY,
        }
    }
}

impl fmt::Display for Flag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Zero => "z",
                Self::Negative => "n",
                Self::HalfCarry => "h",
                Self::Carry => "c",
            }
        )
    }
}

bitflags! {
    #[derive(Copy,Clone,Debug)]
    pub struct Flags: u8 {
        const ZERO = 0b10000000;
        const NEGATIVE = 0b01000000;
        const HALF_CARRY = 0b00100000;
        const CARRY = 0b00010000;

        const _OTHER = !0;
    }
}

impl SerRon for Flags {
    fn ser_ron(&self, _indent_level: usize, state: &mut SerRonState) {
        self.bits().ser_ron(_indent_level, state);
    }
}

impl DeRon for Flags {
    fn de_ron(state: &mut DeRonState, input: &mut std::str::Chars<'_>) -> Result<Self, DeRonErr> {
        Ok(Self::from_bits_retain(u8::de_ron(state, input)?))
    }
}
