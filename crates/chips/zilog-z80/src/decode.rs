//! Opcode field extraction and operand addressing for the algorithmic Z80
//! decode.
//!
//! Every opcode splits into octal fields `x`(7-6) `y`(5-3) `z`(2-0), with
//! `p`(y>>1) and `q`(y&1); the instruction groups key off these. The field
//! values then name registers and register pairs through the tables below,
//! which an index prefix re-points at IX/IY.

use crate::flags;
use crate::{Cpu, InterruptMode};

#[derive(Clone, Copy)]
pub(super) struct Fields {
    pub x: u8,
    pub y: u8,
    pub z: u8,
    pub p: u8,
    pub q: u8,
}

impl Fields {
    pub(super) fn new(opcode: u8) -> Self {
        let y = (opcode >> 3) & 0x07;
        Fields {
            x: opcode >> 6,
            y,
            z: opcode & 0x07,
            p: y >> 1,
            q: y & 1,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum AluOp {
    Add,
    Adc,
    Sub,
    Sbc,
    And,
    Xor,
    Or,
    Cp,
}

impl AluOp {
    pub(super) fn from_index(y: u8) -> Self {
        use AluOp::*;
        [Add, Adc, Sub, Sbc, And, Xor, Or, Cp][y as usize]
    }
}

#[derive(Clone, Copy)]
pub(super) enum RotOp {
    Rlc,
    Rrc,
    Rl,
    Rr,
    Sla,
    Sra,
    Sll,
    Srl,
}

impl RotOp {
    pub(super) fn from_index(y: u8) -> Self {
        use RotOp::*;
        [Rlc, Rrc, Rl, Rr, Sla, Sra, Sll, Srl][y as usize]
    }
}

/// A single register operand, addressing IX/IY halves under a prefix.
#[derive(Clone, Copy)]
pub(super) enum Reg {
    B,
    C,
    D,
    E,
    H,
    L,
    A,
    IxH,
    IxL,
    IyH,
    IyL,
}

/// Which register pair the H/L slot and the (HL) memory operand resolve to —
/// the substitution a DD or FD prefix makes.
#[derive(Clone, Copy)]
pub(super) enum Index {
    Hl,
    Ix,
    Iy,
}

impl Index {
    pub(super) fn base(self, cpu: &Cpu) -> u16 {
        match self {
            Index::Hl => cpu.hl(),
            Index::Ix => cpu.ix,
            Index::Iy => cpu.iy,
        }
    }
}

/// The register an operand index names under `index`.
pub(super) fn reg_at(idx: u8, index: Index) -> Reg {
    match idx {
        0 => Reg::B,
        1 => Reg::C,
        2 => Reg::D,
        3 => Reg::E,
        4 => match index {
            Index::Hl => Reg::H,
            Index::Ix => Reg::IxH,
            Index::Iy => Reg::IyH,
        },
        5 => match index {
            Index::Hl => Reg::L,
            Index::Ix => Reg::IxL,
            Index::Iy => Reg::IyL,
        },
        7 => Reg::A,
        _ => unreachable!(),
    }
}

/// The plain (unsubstituted) register for an operand index — what a memory
/// operand's paired register slot always names.
pub(super) fn real_reg(idx: u8) -> Reg {
    reg_at(idx, Index::Hl)
}

pub(super) const INT_MODE: [InterruptMode; 4] = [
    InterruptMode::Mode0,
    InterruptMode::Mode0,
    InterruptMode::Mode1,
    InterruptMode::Mode2,
];

impl Cpu {
    pub(super) fn condition(&self, index: u8) -> bool {
        match index {
            0 => self.f & flags::ZERO == 0,
            1 => self.f & flags::ZERO != 0,
            2 => self.f & flags::CARRY == 0,
            3 => self.f & flags::CARRY != 0,
            4 => self.f & flags::PARITY == 0,
            5 => self.f & flags::PARITY != 0,
            6 => self.f & flags::SIGN == 0,
            _ => self.f & flags::SIGN != 0,
        }
    }

    /// The rp table: BC, DE, HL/IX/IY, SP.
    pub(super) fn pair(&self, p: u8, index: Index) -> u16 {
        match p {
            0 => self.bc(),
            1 => self.de(),
            2 => index.base(self),
            _ => self.sp,
        }
    }

    pub(super) fn set_pair(&mut self, p: u8, index: Index, value: u16) {
        match p {
            0 => self.set_bc(value),
            1 => self.set_de(value),
            2 => match index {
                Index::Hl => self.set_hl(value),
                Index::Ix => self.ix = value,
                Index::Iy => self.iy = value,
            },
            _ => self.sp = value,
        }
    }

    /// The rp2 table, which PUSH/POP address: AF where rp has SP.
    pub(super) fn stack_pair(&self, p: u8, index: Index) -> u16 {
        match p {
            3 => self.af(),
            _ => self.pair(p, index),
        }
    }

    pub(super) fn set_stack_pair(&mut self, p: u8, index: Index, value: u16) {
        match p {
            3 => [self.a, self.f] = value.to_be_bytes(),
            _ => self.set_pair(p, index, value),
        }
    }

    pub(super) fn reg(&self, r: crate::decode::Reg) -> u8 {
        use Reg::*;
        match r {
            B => self.b,
            C => self.c,
            D => self.d,
            E => self.e,
            H => self.h,
            L => self.l,
            A => self.a,
            IxH => (self.ix >> 8) as u8,
            IxL => self.ix as u8,
            IyH => (self.iy >> 8) as u8,
            IyL => self.iy as u8,
        }
    }

    pub(super) fn set_reg(&mut self, r: crate::decode::Reg, value: u8) {
        use Reg::*;
        match r {
            B => self.b = value,
            C => self.c = value,
            D => self.d = value,
            E => self.e = value,
            H => self.h = value,
            L => self.l = value,
            A => self.a = value,
            IxH => self.ix = (self.ix & 0x00FF) | ((value as u16) << 8),
            IxL => self.ix = (self.ix & 0xFF00) | value as u16,
            IyH => self.iy = (self.iy & 0x00FF) | ((value as u16) << 8),
            IyL => self.iy = (self.iy & 0xFF00) | value as u16,
        }
    }
}
