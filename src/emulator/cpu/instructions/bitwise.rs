use core::fmt;

use super::Source8;

pub enum Bitwise {
    AndA(Source8),
    OrA(Source8),
    XorA(Source8),
    ComplementA,
}

impl Bitwise {
    pub fn decode(op: u8, ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(match op {
            0x2f => Self::ComplementA,
            0xa0 => Self::AndA(Source8::b()),
            0xb0 => Self::OrA(Source8::b()),
            0xa1 => Self::AndA(Source8::c()),
            0xb1 => Self::OrA(Source8::c()),
            0xa2 => Self::AndA(Source8::d()),
            0xb2 => Self::OrA(Source8::d()),
            0xa3 => Self::AndA(Source8::e()),
            0xb3 => Self::OrA(Source8::e()),
            0xa4 => Self::AndA(Source8::h()),
            0xb4 => Self::OrA(Source8::h()),
            0xa5 => Self::AndA(Source8::l()),
            0xb5 => Self::OrA(Source8::l()),
            0xa6 => Self::AndA(Source8::deref_hl()),
            0xb6 => Self::OrA(Source8::deref_hl()),
            0xa7 => Self::AndA(Source8::a()),
            0xb7 => Self::OrA(Source8::a()),
            0xa8 => Self::XorA(Source8::b()),
            0xa9 => Self::XorA(Source8::c()),
            0xaa => Self::XorA(Source8::d()),
            0xab => Self::XorA(Source8::e()),
            0xac => Self::XorA(Source8::h()),
            0xad => Self::XorA(Source8::l()),
            0xae => Self::XorA(Source8::deref_hl()),
            0xaf => Self::XorA(Source8::a()),
            0xe6 => Self::AndA(Source8::constant(ops)?),
            0xf6 => Self::OrA(Source8::constant(ops)?),
            0xee => Self::XorA(Source8::constant(ops)?),

            _ => return None,
        })
    }
}

impl fmt::Display for Bitwise {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::AndA(source) => write!(f, "and a, {}", source),
            Self::OrA(source) => write!(f, "or a, {}", source),
            Self::XorA(source) => write!(f, "xor a, {}", source),
            Self::ComplementA => write!(f, "cpl"),
        }
    }
}
