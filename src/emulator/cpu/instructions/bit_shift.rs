use core::fmt;

use crate::emulator::cpu::instructions::Target8;

#[derive(Clone)]
pub enum BitShift {
    RotateA(Direction, Carry), // Register A optimised variants, not within cb prefix
    Rotate(Direction, Carry, Target8),
    ShiftArithmetical(Direction, Target8),
    ShiftRightLogical(Target8),
    Swap(Target8),
}

#[derive(Clone)]
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

#[derive(Clone)]
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
            0x00 => Self::rlc(Target8::b()),
            0x01 => Self::rlc(Target8::c()),
            0x02 => Self::rlc(Target8::d()),
            0x03 => Self::rlc(Target8::e()),
            0x04 => Self::rlc(Target8::h()),
            0x05 => Self::rlc(Target8::l()),
            0x06 => Self::rlc(Target8::deref_hl()),
            0x07 => Self::rlc(Target8::a()),
            0x08 => Self::rrc(Target8::b()),
            0x09 => Self::rrc(Target8::c()),
            0x0a => Self::rrc(Target8::d()),
            0x0b => Self::rrc(Target8::e()),
            0x0c => Self::rrc(Target8::h()),
            0x0d => Self::rrc(Target8::l()),
            0x0e => Self::rrc(Target8::deref_hl()),
            0x0f => Self::rrc(Target8::a()),
            0x10 => Self::rl(Target8::b()),
            0x11 => Self::rl(Target8::c()),
            0x12 => Self::rl(Target8::d()),
            0x13 => Self::rl(Target8::e()),
            0x14 => Self::rl(Target8::h()),
            0x15 => Self::rl(Target8::l()),
            0x16 => Self::rl(Target8::deref_hl()),
            0x17 => Self::rl(Target8::a()),
            0x18 => Self::rr(Target8::b()),
            0x19 => Self::rr(Target8::c()),
            0x1a => Self::rr(Target8::d()),
            0x1b => Self::rr(Target8::e()),
            0x1c => Self::rr(Target8::h()),
            0x1d => Self::rr(Target8::l()),
            0x1e => Self::rr(Target8::deref_hl()),
            0x1f => Self::rr(Target8::a()),
            0x20 => Self::sla(Target8::b()),
            0x21 => Self::sla(Target8::c()),
            0x22 => Self::sla(Target8::d()),
            0x23 => Self::sla(Target8::e()),
            0x24 => Self::sla(Target8::h()),
            0x25 => Self::sla(Target8::l()),
            0x26 => Self::sla(Target8::deref_hl()),
            0x27 => Self::sla(Target8::a()),
            0x28 => Self::sra(Target8::b()),
            0x29 => Self::sra(Target8::c()),
            0x2a => Self::sra(Target8::d()),
            0x2b => Self::sra(Target8::e()),
            0x2c => Self::sra(Target8::h()),
            0x2d => Self::sra(Target8::l()),
            0x2e => Self::sra(Target8::deref_hl()),
            0x2f => Self::sra(Target8::a()),

            0x30 => Self::Swap(Target8::b()),
            0x31 => Self::Swap(Target8::c()),
            0x32 => Self::Swap(Target8::d()),
            0x33 => Self::Swap(Target8::e()),
            0x34 => Self::Swap(Target8::h()),
            0x35 => Self::Swap(Target8::l()),
            0x36 => Self::Swap(Target8::deref_hl()),
            0x37 => Self::Swap(Target8::a()),
            0x38 => Self::ShiftRightLogical(Target8::b()),
            0x39 => Self::ShiftRightLogical(Target8::c()),
            0x3a => Self::ShiftRightLogical(Target8::d()),
            0x3b => Self::ShiftRightLogical(Target8::e()),
            0x3c => Self::ShiftRightLogical(Target8::h()),
            0x3d => Self::ShiftRightLogical(Target8::l()),
            0x3e => Self::ShiftRightLogical(Target8::deref_hl()),
            0x3f => Self::ShiftRightLogical(Target8::a()),

            _ => unreachable!(),
        }
    }

    fn rlc(target: Target8) -> Self {
        Self::Rotate(Direction::Left, Carry::SetOnly, target)
    }

    fn rrc(target: Target8) -> Self {
        Self::Rotate(Direction::Right, Carry::SetOnly, target)
    }

    fn rl(target: Target8) -> Self {
        Self::Rotate(Direction::Left, Carry::Through, target)
    }

    fn rr(target: Target8) -> Self {
        Self::Rotate(Direction::Right, Carry::Through, target)
    }

    fn sla(target: Target8) -> Self {
        Self::ShiftArithmetical(Direction::Left, target)
    }

    fn sra(target: Target8) -> Self {
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
