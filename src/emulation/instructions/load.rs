use crate::emulation::instructions::{Source8, Source16, Target8, Target16};
use core::fmt;

pub enum Load {
    Load8(Target8, Source8),
    Load16(Target16, Source16),
}

impl Load {
    pub fn decode(op: u8, ops: &mut impl Iterator<Item = u8>) -> Option<Self> {
        Some(match op {
            0x02 => Self::Load8(Target8::deref_bc(), Source8::a()),
            0x12 => Self::Load8(Target8::deref_de(), Source8::a()),
            0x22 => Self::Load8(Target8::deref_hl_inc(), Source8::a()),
            0x32 => Self::Load8(Target8::deref_hl_dec(), Source8::a()),
            0x06 => Self::Load8(Target8::b(), Source8::constant(ops)?),
            0x16 => Self::Load8(Target8::d(), Source8::constant(ops)?),
            0x26 => Self::Load8(Target8::h(), Source8::constant(ops)?),
            0x36 => Self::Load8(Target8::deref_hl(), Source8::constant(ops)?),
            0x0a => Self::Load8(Target8::a(), Source8::deref_bc()),
            0x1a => Self::Load8(Target8::a(), Source8::deref_de()),
            0x2a => Self::Load8(Target8::a(), Source8::deref_hl_inc()),
            0x3a => Self::Load8(Target8::a(), Source8::deref_hl_dec()),
            0x0e => Self::Load8(Target8::c(), Source8::constant(ops)?),
            0x1e => Self::Load8(Target8::e(), Source8::constant(ops)?),
            0x2e => Self::Load8(Target8::l(), Source8::constant(ops)?),
            0x3e => Self::Load8(Target8::a(), Source8::constant(ops)?),
            0x40 => Self::Load8(Target8::b(), Source8::b()),
            0x50 => Self::Load8(Target8::d(), Source8::b()),
            0x60 => Self::Load8(Target8::h(), Source8::b()),
            0x70 => Self::Load8(Target8::deref_hl(), Source8::b()),
            0x41 => Self::Load8(Target8::b(), Source8::c()),
            0x51 => Self::Load8(Target8::d(), Source8::c()),
            0x61 => Self::Load8(Target8::h(), Source8::c()),
            0x71 => Self::Load8(Target8::deref_hl(), Source8::c()),
            0x42 => Self::Load8(Target8::b(), Source8::d()),
            0x52 => Self::Load8(Target8::d(), Source8::d()),
            0x62 => Self::Load8(Target8::h(), Source8::d()),
            0x72 => Self::Load8(Target8::deref_hl(), Source8::d()),
            0x43 => Self::Load8(Target8::b(), Source8::e()),
            0x53 => Self::Load8(Target8::d(), Source8::e()),
            0x63 => Self::Load8(Target8::h(), Source8::e()),
            0x73 => Self::Load8(Target8::deref_hl(), Source8::e()),
            0x44 => Self::Load8(Target8::b(), Source8::h()),
            0x54 => Self::Load8(Target8::d(), Source8::h()),
            0x64 => Self::Load8(Target8::h(), Source8::h()),
            0x74 => Self::Load8(Target8::deref_hl(), Source8::h()),
            0x45 => Self::Load8(Target8::b(), Source8::l()),
            0x55 => Self::Load8(Target8::d(), Source8::l()),
            0x65 => Self::Load8(Target8::h(), Source8::l()),
            0x75 => Self::Load8(Target8::deref_hl(), Source8::l()),
            0x46 => Self::Load8(Target8::b(), Source8::deref_hl()),
            0x56 => Self::Load8(Target8::d(), Source8::deref_hl()),
            0x66 => Self::Load8(Target8::h(), Source8::deref_hl()),
            0x47 => Self::Load8(Target8::b(), Source8::a()),
            0x57 => Self::Load8(Target8::d(), Source8::a()),
            0x67 => Self::Load8(Target8::h(), Source8::a()),
            0x77 => Self::Load8(Target8::deref_hl(), Source8::a()),
            0x48 => Self::Load8(Target8::c(), Source8::b()),
            0x58 => Self::Load8(Target8::e(), Source8::b()),
            0x68 => Self::Load8(Target8::l(), Source8::b()),
            0x78 => Self::Load8(Target8::a(), Source8::b()),
            0x49 => Self::Load8(Target8::c(), Source8::c()),
            0x59 => Self::Load8(Target8::e(), Source8::c()),
            0x69 => Self::Load8(Target8::l(), Source8::c()),
            0x79 => Self::Load8(Target8::a(), Source8::c()),
            0x4a => Self::Load8(Target8::c(), Source8::d()),
            0x5a => Self::Load8(Target8::e(), Source8::d()),
            0x6a => Self::Load8(Target8::l(), Source8::d()),
            0x7a => Self::Load8(Target8::a(), Source8::d()),
            0x4b => Self::Load8(Target8::c(), Source8::e()),
            0x5b => Self::Load8(Target8::e(), Source8::e()),
            0x6b => Self::Load8(Target8::l(), Source8::e()),
            0x7b => Self::Load8(Target8::a(), Source8::e()),
            0x4c => Self::Load8(Target8::c(), Source8::h()),
            0x5c => Self::Load8(Target8::e(), Source8::h()),
            0x6c => Self::Load8(Target8::l(), Source8::h()),
            0x7c => Self::Load8(Target8::a(), Source8::h()),
            0x4d => Self::Load8(Target8::c(), Source8::l()),
            0x5d => Self::Load8(Target8::e(), Source8::l()),
            0x6d => Self::Load8(Target8::l(), Source8::l()),
            0x7d => Self::Load8(Target8::a(), Source8::l()),
            0x4e => Self::Load8(Target8::c(), Source8::deref_hl()),
            0x5e => Self::Load8(Target8::e(), Source8::deref_hl()),
            0x6e => Self::Load8(Target8::l(), Source8::deref_hl()),
            0x7e => Self::Load8(Target8::a(), Source8::deref_hl()),
            0x4f => Self::Load8(Target8::c(), Source8::a()),
            0x5f => Self::Load8(Target8::e(), Source8::a()),
            0x6f => Self::Load8(Target8::l(), Source8::a()),
            0x7f => Self::Load8(Target8::a(), Source8::a()),
            0xe0 => Self::Load8(Target8::hram(ops)?, Source8::a()),
            0xf0 => Self::Load8(Target8::a(), Source8::hram(ops)?),
            0xe2 => Self::Load8(Target8::hram_c(), Source8::a()),
            0xf2 => Self::Load8(Target8::a(), Source8::hram_c()),
            0xea => Self::Load8(Target8::deref_fixed(ops)?, Source8::a()),
            0xfa => Self::Load8(Target8::a(), Source8::deref_fixed(ops)?),

            0x01 => Self::Load16(Target16::bc(), Source16::constant(ops)?),
            0x11 => Self::Load16(Target16::de(), Source16::constant(ops)?),
            0x21 => Self::Load16(Target16::hl(), Source16::constant(ops)?),
            0x31 => Self::Load16(Target16::sp(), Source16::constant(ops)?),
            0x08 => Self::Load16(Target16::memory(ops)?, Source16::sp()),
            0xf8 => Self::Load16(Target16::hl(), Source16::sp_with_offset(ops)?),
            0xf9 => Self::Load16(Target16::sp(), Source16::hl()),

            _ => return None,
        })
    }
}

impl fmt::Display for Load {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Load::Load8(target, source) => {
                write!(f, "ld {}, {}", target, source)
            }

            Load::Load16(target, source) => {
                write!(f, "ld {}, {}", target, source)
            }
        }
    }
}
