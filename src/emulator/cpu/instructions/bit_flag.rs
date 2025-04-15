use core::fmt;

use super::{Source8, Target8};

#[derive(Clone)]
pub enum BitFlag {
    Check(u8, Source8),
    Set(u8, Target8),
    Unset(u8, Target8),
}

impl BitFlag {
    pub fn decode(op: u8) -> Self {
        match op {
            0x40 => Self::Check(0, Source8::b()),
            0x41 => Self::Check(0, Source8::c()),
            0x42 => Self::Check(0, Source8::d()),
            0x43 => Self::Check(0, Source8::e()),
            0x44 => Self::Check(0, Source8::h()),
            0x45 => Self::Check(0, Source8::l()),
            0x46 => Self::Check(0, Source8::deref_hl()),
            0x47 => Self::Check(0, Source8::a()),
            0x48 => Self::Check(1, Source8::b()),
            0x49 => Self::Check(1, Source8::c()),
            0x4a => Self::Check(1, Source8::d()),
            0x4b => Self::Check(1, Source8::e()),
            0x4c => Self::Check(1, Source8::h()),
            0x4d => Self::Check(1, Source8::l()),
            0x4e => Self::Check(1, Source8::deref_hl()),
            0x4f => Self::Check(1, Source8::a()),
            0x50 => Self::Check(2, Source8::b()),
            0x51 => Self::Check(2, Source8::c()),
            0x52 => Self::Check(2, Source8::d()),
            0x53 => Self::Check(2, Source8::e()),
            0x54 => Self::Check(2, Source8::h()),
            0x55 => Self::Check(2, Source8::l()),
            0x56 => Self::Check(2, Source8::deref_hl()),
            0x57 => Self::Check(2, Source8::a()),
            0x58 => Self::Check(3, Source8::b()),
            0x59 => Self::Check(3, Source8::c()),
            0x5a => Self::Check(3, Source8::d()),
            0x5b => Self::Check(3, Source8::e()),
            0x5c => Self::Check(3, Source8::h()),
            0x5d => Self::Check(3, Source8::l()),
            0x5e => Self::Check(3, Source8::deref_hl()),
            0x5f => Self::Check(3, Source8::a()),
            0x60 => Self::Check(4, Source8::b()),
            0x61 => Self::Check(4, Source8::c()),
            0x62 => Self::Check(4, Source8::d()),
            0x63 => Self::Check(4, Source8::e()),
            0x64 => Self::Check(4, Source8::h()),
            0x65 => Self::Check(4, Source8::l()),
            0x66 => Self::Check(4, Source8::deref_hl()),
            0x67 => Self::Check(4, Source8::a()),
            0x68 => Self::Check(5, Source8::b()),
            0x69 => Self::Check(5, Source8::c()),
            0x6a => Self::Check(5, Source8::d()),
            0x6b => Self::Check(5, Source8::e()),
            0x6c => Self::Check(5, Source8::h()),
            0x6d => Self::Check(5, Source8::l()),
            0x6e => Self::Check(5, Source8::deref_hl()),
            0x6f => Self::Check(5, Source8::a()),
            0x70 => Self::Check(6, Source8::b()),
            0x71 => Self::Check(6, Source8::c()),
            0x72 => Self::Check(6, Source8::d()),
            0x73 => Self::Check(6, Source8::e()),
            0x74 => Self::Check(6, Source8::h()),
            0x75 => Self::Check(6, Source8::l()),
            0x76 => Self::Check(6, Source8::deref_hl()),
            0x77 => Self::Check(6, Source8::a()),
            0x78 => Self::Check(7, Source8::b()),
            0x79 => Self::Check(7, Source8::c()),
            0x7a => Self::Check(7, Source8::d()),
            0x7b => Self::Check(7, Source8::e()),
            0x7c => Self::Check(7, Source8::h()),
            0x7d => Self::Check(7, Source8::l()),
            0x7e => Self::Check(7, Source8::deref_hl()),
            0x7f => Self::Check(7, Source8::a()),
            0x80 => Self::Unset(0, Target8::b()),
            0x81 => Self::Unset(0, Target8::c()),
            0x82 => Self::Unset(0, Target8::d()),
            0x83 => Self::Unset(0, Target8::e()),
            0x84 => Self::Unset(0, Target8::h()),
            0x85 => Self::Unset(0, Target8::l()),
            0x86 => Self::Unset(0, Target8::deref_hl()),
            0x87 => Self::Unset(0, Target8::a()),
            0x88 => Self::Unset(1, Target8::b()),
            0x89 => Self::Unset(1, Target8::c()),
            0x8a => Self::Unset(1, Target8::d()),
            0x8b => Self::Unset(1, Target8::e()),
            0x8c => Self::Unset(1, Target8::h()),
            0x8d => Self::Unset(1, Target8::l()),
            0x8e => Self::Unset(1, Target8::deref_hl()),
            0x8f => Self::Unset(1, Target8::a()),
            0x90 => Self::Unset(2, Target8::b()),
            0x91 => Self::Unset(2, Target8::c()),
            0x92 => Self::Unset(2, Target8::d()),
            0x93 => Self::Unset(2, Target8::e()),
            0x94 => Self::Unset(2, Target8::h()),
            0x95 => Self::Unset(2, Target8::l()),
            0x96 => Self::Unset(2, Target8::deref_hl()),
            0x97 => Self::Unset(2, Target8::a()),
            0x98 => Self::Unset(3, Target8::b()),
            0x99 => Self::Unset(3, Target8::c()),
            0x9a => Self::Unset(3, Target8::d()),
            0x9b => Self::Unset(3, Target8::e()),
            0x9c => Self::Unset(3, Target8::h()),
            0x9d => Self::Unset(3, Target8::l()),
            0x9e => Self::Unset(3, Target8::deref_hl()),
            0x9f => Self::Unset(3, Target8::a()),
            0xa0 => Self::Unset(4, Target8::b()),
            0xa1 => Self::Unset(4, Target8::c()),
            0xa2 => Self::Unset(4, Target8::d()),
            0xa3 => Self::Unset(4, Target8::e()),
            0xa4 => Self::Unset(4, Target8::h()),
            0xa5 => Self::Unset(4, Target8::l()),
            0xa6 => Self::Unset(4, Target8::deref_hl()),
            0xa7 => Self::Unset(4, Target8::a()),
            0xa8 => Self::Unset(5, Target8::b()),
            0xa9 => Self::Unset(5, Target8::c()),
            0xaa => Self::Unset(5, Target8::d()),
            0xab => Self::Unset(5, Target8::e()),
            0xac => Self::Unset(5, Target8::h()),
            0xad => Self::Unset(5, Target8::l()),
            0xae => Self::Unset(5, Target8::deref_hl()),
            0xaf => Self::Unset(5, Target8::a()),
            0xb0 => Self::Unset(6, Target8::b()),
            0xb1 => Self::Unset(6, Target8::c()),
            0xb2 => Self::Unset(6, Target8::d()),
            0xb3 => Self::Unset(6, Target8::e()),
            0xb4 => Self::Unset(6, Target8::h()),
            0xb5 => Self::Unset(6, Target8::l()),
            0xb6 => Self::Unset(6, Target8::deref_hl()),
            0xb7 => Self::Unset(6, Target8::a()),
            0xb8 => Self::Unset(7, Target8::b()),
            0xb9 => Self::Unset(7, Target8::c()),
            0xba => Self::Unset(7, Target8::d()),
            0xbb => Self::Unset(7, Target8::e()),
            0xbc => Self::Unset(7, Target8::h()),
            0xbd => Self::Unset(7, Target8::l()),
            0xbe => Self::Unset(7, Target8::deref_hl()),
            0xbf => Self::Unset(7, Target8::a()),
            0xc0 => Self::Set(0, Target8::b()),
            0xc1 => Self::Set(0, Target8::c()),
            0xc2 => Self::Set(0, Target8::d()),
            0xc3 => Self::Set(0, Target8::e()),
            0xc4 => Self::Set(0, Target8::h()),
            0xc5 => Self::Set(0, Target8::l()),
            0xc6 => Self::Set(0, Target8::deref_hl()),
            0xc7 => Self::Set(0, Target8::a()),
            0xc8 => Self::Set(1, Target8::b()),
            0xc9 => Self::Set(1, Target8::c()),
            0xca => Self::Set(1, Target8::d()),
            0xcb => Self::Set(1, Target8::e()),
            0xcc => Self::Set(1, Target8::h()),
            0xcd => Self::Set(1, Target8::l()),
            0xce => Self::Set(1, Target8::deref_hl()),
            0xcf => Self::Set(1, Target8::a()),
            0xd0 => Self::Set(2, Target8::b()),
            0xd1 => Self::Set(2, Target8::c()),
            0xd2 => Self::Set(2, Target8::d()),
            0xd3 => Self::Set(2, Target8::e()),
            0xd4 => Self::Set(2, Target8::h()),
            0xd5 => Self::Set(2, Target8::l()),
            0xd6 => Self::Set(2, Target8::deref_hl()),
            0xd7 => Self::Set(2, Target8::a()),
            0xd8 => Self::Set(3, Target8::b()),
            0xd9 => Self::Set(3, Target8::c()),
            0xda => Self::Set(3, Target8::d()),
            0xdb => Self::Set(3, Target8::e()),
            0xdc => Self::Set(3, Target8::h()),
            0xdd => Self::Set(3, Target8::l()),
            0xde => Self::Set(3, Target8::deref_hl()),
            0xdf => Self::Set(3, Target8::a()),
            0xe0 => Self::Set(4, Target8::b()),
            0xe1 => Self::Set(4, Target8::c()),
            0xe2 => Self::Set(4, Target8::d()),
            0xe3 => Self::Set(4, Target8::e()),
            0xe4 => Self::Set(4, Target8::h()),
            0xe5 => Self::Set(4, Target8::l()),
            0xe6 => Self::Set(4, Target8::deref_hl()),
            0xe7 => Self::Set(4, Target8::a()),
            0xe8 => Self::Set(5, Target8::b()),
            0xe9 => Self::Set(5, Target8::c()),
            0xea => Self::Set(5, Target8::d()),
            0xeb => Self::Set(5, Target8::e()),
            0xec => Self::Set(5, Target8::h()),
            0xed => Self::Set(5, Target8::l()),
            0xee => Self::Set(5, Target8::deref_hl()),
            0xef => Self::Set(5, Target8::a()),
            0xf0 => Self::Set(6, Target8::b()),
            0xf1 => Self::Set(6, Target8::c()),
            0xf2 => Self::Set(6, Target8::d()),
            0xf3 => Self::Set(6, Target8::e()),
            0xf4 => Self::Set(6, Target8::h()),
            0xf5 => Self::Set(6, Target8::l()),
            0xf6 => Self::Set(6, Target8::deref_hl()),
            0xf7 => Self::Set(6, Target8::a()),
            0xf8 => Self::Set(7, Target8::b()),
            0xf9 => Self::Set(7, Target8::c()),
            0xfa => Self::Set(7, Target8::d()),
            0xfb => Self::Set(7, Target8::e()),
            0xfc => Self::Set(7, Target8::h()),
            0xfd => Self::Set(7, Target8::l()),
            0xfe => Self::Set(7, Target8::deref_hl()),
            0xff => Self::Set(7, Target8::a()),
            _ => unreachable!(),
        }
    }
}

impl fmt::Display for BitFlag {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Check(index, source) => write!(f, "bit {}, {}", index, source),
            Self::Set(index, source) => write!(f, "set {}, {}", index, source),
            Self::Unset(index, source) => write!(f, "res {}, {}", index, source),
        }
    }
}
