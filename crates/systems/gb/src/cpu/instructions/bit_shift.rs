use core::fmt;

use crate::cpu::instructions::Place8;

#[derive(Clone)]
pub enum BitShift {
    RotateA(Direction, Carry), // Register A optimised variants, not within cb prefix
    Rotate(Direction, Carry, Place8),
    ShiftArithmetical(Direction, Place8),
    ShiftRightLogical(Place8),
    Swap(Place8),
}

#[derive(Clone, Debug)]
pub enum Direction {
    Left,
    Right,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Left => write!(f, "l"),
            Self::Right => write!(f, "r"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Carry {
    Through,
    SetOnly,
}

impl fmt::Display for Carry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Through => write!(f, ""),
            Self::SetOnly => write!(f, "c"),
        }
    }
}

impl BitShift {
    pub fn decode(op: u8) -> Self {
        match op {
            0x00 => Self::rlc(Place8::b()),
            0x01 => Self::rlc(Place8::c()),
            0x02 => Self::rlc(Place8::d()),
            0x03 => Self::rlc(Place8::e()),
            0x04 => Self::rlc(Place8::h()),
            0x05 => Self::rlc(Place8::l()),
            0x06 => Self::rlc(Place8::deref_hl()),
            0x07 => Self::rlc(Place8::a()),
            0x08 => Self::rrc(Place8::b()),
            0x09 => Self::rrc(Place8::c()),
            0x0a => Self::rrc(Place8::d()),
            0x0b => Self::rrc(Place8::e()),
            0x0c => Self::rrc(Place8::h()),
            0x0d => Self::rrc(Place8::l()),
            0x0e => Self::rrc(Place8::deref_hl()),
            0x0f => Self::rrc(Place8::a()),
            0x10 => Self::rl(Place8::b()),
            0x11 => Self::rl(Place8::c()),
            0x12 => Self::rl(Place8::d()),
            0x13 => Self::rl(Place8::e()),
            0x14 => Self::rl(Place8::h()),
            0x15 => Self::rl(Place8::l()),
            0x16 => Self::rl(Place8::deref_hl()),
            0x17 => Self::rl(Place8::a()),
            0x18 => Self::rr(Place8::b()),
            0x19 => Self::rr(Place8::c()),
            0x1a => Self::rr(Place8::d()),
            0x1b => Self::rr(Place8::e()),
            0x1c => Self::rr(Place8::h()),
            0x1d => Self::rr(Place8::l()),
            0x1e => Self::rr(Place8::deref_hl()),
            0x1f => Self::rr(Place8::a()),
            0x20 => Self::sla(Place8::b()),
            0x21 => Self::sla(Place8::c()),
            0x22 => Self::sla(Place8::d()),
            0x23 => Self::sla(Place8::e()),
            0x24 => Self::sla(Place8::h()),
            0x25 => Self::sla(Place8::l()),
            0x26 => Self::sla(Place8::deref_hl()),
            0x27 => Self::sla(Place8::a()),
            0x28 => Self::sra(Place8::b()),
            0x29 => Self::sra(Place8::c()),
            0x2a => Self::sra(Place8::d()),
            0x2b => Self::sra(Place8::e()),
            0x2c => Self::sra(Place8::h()),
            0x2d => Self::sra(Place8::l()),
            0x2e => Self::sra(Place8::deref_hl()),
            0x2f => Self::sra(Place8::a()),

            0x30 => Self::Swap(Place8::b()),
            0x31 => Self::Swap(Place8::c()),
            0x32 => Self::Swap(Place8::d()),
            0x33 => Self::Swap(Place8::e()),
            0x34 => Self::Swap(Place8::h()),
            0x35 => Self::Swap(Place8::l()),
            0x36 => Self::Swap(Place8::deref_hl()),
            0x37 => Self::Swap(Place8::a()),
            0x38 => Self::ShiftRightLogical(Place8::b()),
            0x39 => Self::ShiftRightLogical(Place8::c()),
            0x3a => Self::ShiftRightLogical(Place8::d()),
            0x3b => Self::ShiftRightLogical(Place8::e()),
            0x3c => Self::ShiftRightLogical(Place8::h()),
            0x3d => Self::ShiftRightLogical(Place8::l()),
            0x3e => Self::ShiftRightLogical(Place8::deref_hl()),
            0x3f => Self::ShiftRightLogical(Place8::a()),

            _ => unreachable!(),
        }
    }

    fn rlc(target: Place8) -> Self {
        Self::Rotate(Direction::Left, Carry::SetOnly, target)
    }

    fn rrc(target: Place8) -> Self {
        Self::Rotate(Direction::Right, Carry::SetOnly, target)
    }

    fn rl(target: Place8) -> Self {
        Self::Rotate(Direction::Left, Carry::Through, target)
    }

    fn rr(target: Place8) -> Self {
        Self::Rotate(Direction::Right, Carry::Through, target)
    }

    fn sla(target: Place8) -> Self {
        Self::ShiftArithmetical(Direction::Left, target)
    }

    fn sra(target: Place8) -> Self {
        Self::ShiftArithmetical(Direction::Right, target)
    }
}

impl fmt::Display for BitShift {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::RotateA(direction, carry) => write!(f, "r{}{}a", direction, carry),
            Self::Rotate(direction, carry, target) => {
                write!(f, "r{}{} {}", direction, carry, target)
            }
            Self::ShiftArithmetical(direction, target) => {
                write!(f, "s{}a {}", direction, target)
            }
            Self::ShiftRightLogical(target) => write!(f, "srl {}", target),
            Self::Swap(target) => write!(f, "swap {}", target),
        }
    }
}
