//! Opcode field extraction for the algorithmic Z80 decode.
//!
//! Every opcode splits into octal fields `x`(7-6) `y`(5-3) `z`(2-0), with
//! `p`(y>>1) and `q`(y&1); the instruction groups key off these.

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

#[derive(Clone, Copy, PartialEq, Eq)]
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

#[derive(Clone, Copy, PartialEq, Eq)]
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
#[derive(Clone, Copy, PartialEq, Eq)]
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
