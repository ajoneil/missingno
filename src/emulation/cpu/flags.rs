use bitflags::bitflags;
use core::fmt;

pub enum Flag {
    Zero,
    Negative,
    HalfCarry,
    Carry,
}

impl Into<Flags> for Flag {
    fn into(self) -> Flags {
        match self {
            Self::Zero => Flags::ZERO,
            Self::Negative => Flags::NEGATIVE,
            Self::HalfCarry => Flags::HALF_CARRY,
            Self::Carry => Flags::CARRY,
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

bitflags! {
    #[derive(Copy,Clone,Debug)]
    pub struct Interrupts: u8 {
        const JOYPAD = 0b00010000;
        const SERIAL = 0b00001000;
        const TIMER  = 0b00000100;
        const LCD    = 0b00000010;
        const VBLANK = 0b00000001;

        const _OTHER = !0;
    }
}
