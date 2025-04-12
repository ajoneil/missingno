use super::{Address, Instruction};
use crate::emulation::cpu::Flag;
use core::fmt;

pub enum Jump {
    Jump(Option<Condition>, Location),
    Call(Option<Condition>, Location),
    Return(Option<Condition>),
    ReturnAndEnableInterrupts,
    Restart(u8),
}

pub enum Location {
    Address(Address),
    RegisterHl,
}

impl Location {
    pub fn fixed(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Address(Address::fixed(ops)?))
    }

    pub fn relative(ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(Self::Address(Address::relative(ops)?))
    }

    pub fn hl() -> Self {
        Self::RegisterHl
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Location::Address(address) => address.to_string(),
                Location::RegisterHl => "hl".to_string(),
            }
        )
    }
}

pub struct Condition(pub Flag, pub bool);

impl Condition {
    pub fn z() -> Option<Self> {
        Some(Self(Flag::Zero, true))
    }

    pub fn nz() -> Option<Self> {
        Some(Self(Flag::Zero, false))
    }

    pub fn c() -> Option<Self> {
        Some(Self(Flag::Carry, true))
    }

    pub fn nc() -> Option<Self> {
        Some(Self(Flag::Carry, false))
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", if self.1 { "" } else { "n" }, self.0)
    }
}

impl Jump {
    pub fn decode(op: u8, ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(match op {
            0x20 => Self::Jump(Condition::nz(), Location::relative(ops)?),
            0x30 => Self::Jump(Condition::nc(), Location::relative(ops)?),
            0x18 => Self::Jump(None, Location::relative(ops)?),
            0x28 => Self::Jump(Condition::z(), Location::relative(ops)?),
            0x38 => Self::Jump(Condition::c(), Location::relative(ops)?),
            0xc0 => Self::Return(Condition::nz()),
            0xd0 => Self::Return(Condition::nc()),
            0xc2 => Self::Jump(Condition::nz(), Location::fixed(ops)?),
            0xd2 => Self::Jump(Condition::nc(), Location::fixed(ops)?),
            0xc3 => Self::Jump(None, Location::fixed(ops)?),
            0xc4 => Self::Call(Condition::nz(), Location::fixed(ops)?),
            0xd4 => Self::Call(Condition::nc(), Location::fixed(ops)?),
            0xc6 => Self::Restart(0x00),
            0xd6 => Self::Restart(0x10),
            0xe6 => Self::Restart(0x20),
            0xf6 => Self::Restart(0x30),
            0xc8 => Self::Return(Condition::z()),
            0xd8 => Self::Return(Condition::c()),
            0xc9 => Self::Return(None),
            0xd9 => Self::ReturnAndEnableInterrupts,
            0xe9 => Self::Jump(None, Location::hl()),
            0xca => Self::Jump(Condition::z(), Location::fixed(ops)?),
            0xda => Self::Jump(Condition::c(), Location::fixed(ops)?),
            0xcc => Self::Call(Condition::z(), Location::fixed(ops)?),
            0xdc => Self::Call(Condition::c(), Location::fixed(ops)?),
            0xcd => Self::Call(None, Location::fixed(ops)?),
            0xcf => Self::Restart(0x08),
            0xdf => Self::Restart(0x18),
            0xef => Self::Restart(0x28),
            0xff => Self::Restart(0x38),
            _ => return None,
        })
    }
}

impl fmt::Display for Jump {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Jump(condition, location) => write!(
                f,
                "jp {}{}",
                match condition {
                    Some(condition) => format!("{}, ", condition),
                    None => "".to_string(),
                },
                location
            ),
            Self::Call(condition, location) => write!(
                f,
                "call {}{}",
                match condition {
                    Some(condition) => format!("{}, ", condition),
                    None => "".to_string(),
                },
                location
            ),

            Self::Return(condition) => write!(
                f,
                "ret{}",
                match condition {
                    Some(condition) => format!(" {}", condition),
                    None => "".to_string(),
                },
            ),

            Self::ReturnAndEnableInterrupts => write!(f, "reti"),

            Self::Restart(address) => write!(f, "rst ${:2x}", address),
        }
    }
}

impl Into<Instruction> for Jump {
    fn into(self) -> Instruction {
        Instruction::Jump(self)
    }
}
