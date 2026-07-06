//! NMOS 6502 opcode decode table — all 256 opcodes, documented and illegal.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Implied,
    Accumulator,
    Immediate,
    ZeroPage,
    ZeroPageX,
    ZeroPageY,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    IndirectX,
    IndirectY,
    Relative,
    Indirect,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flag {
    Carry,
    Zero,
    Negative,
    Overflow,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Lda,
    Ldx,
    Ldy,
    Sta,
    Stx,
    Sty,
    Tax,
    Tay,
    Txa,
    Tya,
    Tsx,
    Txs,
    Pha,
    Php,
    Pla,
    Plp,
    Adc,
    Sbc,
    And,
    Ora,
    Eor,
    Cmp,
    Cpx,
    Cpy,
    Bit,
    Asl,
    Lsr,
    Rol,
    Ror,
    Inc,
    Dec,
    Inx,
    Iny,
    Dex,
    Dey,
    Clc,
    Sec,
    Cli,
    Sei,
    Clv,
    Cld,
    Sed,
    Jmp,
    Jsr,
    Rts,
    Rti,
    Brk,
    Nop,
    Branch(Flag, bool),
    // Illegal opcodes.
    Lax,
    Sax,
    Dcp,
    Isc,
    Slo,
    Rla,
    Sre,
    Rra,
    Anc,
    Alr,
    Arr,
    Ane,
    Lxa,
    Sbx,
    Sha,
    Shx,
    Shy,
    Tas,
    Las,
    Jam,
}

#[derive(Clone, Copy, Debug)]
pub struct Instr {
    pub op: Op,
    pub mode: Mode,
}

/// How an instruction uses its resolved operand — decides the cycle sequence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    Read,
    Write,
    ReadModifyWrite,
}

impl Op {
    pub fn access(self) -> Access {
        use Op::*;
        match self {
            Sta | Stx | Sty | Sax | Sha | Shx | Shy | Tas => Access::Write,
            Asl | Lsr | Rol | Ror | Inc | Dec | Dcp | Isc | Slo | Rla | Sre | Rra => {
                Access::ReadModifyWrite
            }
            _ => Access::Read,
        }
    }
}

const fn i(op: Op, mode: Mode) -> Instr {
    Instr { op, mode }
}

use Flag::*;
use Mode::*;
use Op::*;

#[rustfmt::skip]
pub const DECODE: [Instr; 256] = [
    // 0x00
    i(Brk, Implied), i(Ora, IndirectX), i(Jam, Implied), i(Slo, IndirectX),
    i(Nop, ZeroPage), i(Ora, ZeroPage), i(Asl, ZeroPage), i(Slo, ZeroPage),
    i(Php, Implied), i(Ora, Immediate), i(Asl, Accumulator), i(Anc, Immediate),
    i(Nop, Absolute), i(Ora, Absolute), i(Asl, Absolute), i(Slo, Absolute),
    // 0x10
    i(Branch(Negative, false), Relative), i(Ora, IndirectY), i(Jam, Implied), i(Slo, IndirectY),
    i(Nop, ZeroPageX), i(Ora, ZeroPageX), i(Asl, ZeroPageX), i(Slo, ZeroPageX),
    i(Clc, Implied), i(Ora, AbsoluteY), i(Nop, Implied), i(Slo, AbsoluteY),
    i(Nop, AbsoluteX), i(Ora, AbsoluteX), i(Asl, AbsoluteX), i(Slo, AbsoluteX),
    // 0x20
    i(Jsr, Absolute), i(And, IndirectX), i(Jam, Implied), i(Rla, IndirectX),
    i(Bit, ZeroPage), i(And, ZeroPage), i(Rol, ZeroPage), i(Rla, ZeroPage),
    i(Plp, Implied), i(And, Immediate), i(Rol, Accumulator), i(Anc, Immediate),
    i(Bit, Absolute), i(And, Absolute), i(Rol, Absolute), i(Rla, Absolute),
    // 0x30
    i(Branch(Negative, true), Relative), i(And, IndirectY), i(Jam, Implied), i(Rla, IndirectY),
    i(Nop, ZeroPageX), i(And, ZeroPageX), i(Rol, ZeroPageX), i(Rla, ZeroPageX),
    i(Sec, Implied), i(And, AbsoluteY), i(Nop, Implied), i(Rla, AbsoluteY),
    i(Nop, AbsoluteX), i(And, AbsoluteX), i(Rol, AbsoluteX), i(Rla, AbsoluteX),
    // 0x40
    i(Rti, Implied), i(Eor, IndirectX), i(Jam, Implied), i(Sre, IndirectX),
    i(Nop, ZeroPage), i(Eor, ZeroPage), i(Lsr, ZeroPage), i(Sre, ZeroPage),
    i(Pha, Implied), i(Eor, Immediate), i(Lsr, Accumulator), i(Alr, Immediate),
    i(Jmp, Absolute), i(Eor, Absolute), i(Lsr, Absolute), i(Sre, Absolute),
    // 0x50
    i(Branch(Overflow, false), Relative), i(Eor, IndirectY), i(Jam, Implied), i(Sre, IndirectY),
    i(Nop, ZeroPageX), i(Eor, ZeroPageX), i(Lsr, ZeroPageX), i(Sre, ZeroPageX),
    i(Cli, Implied), i(Eor, AbsoluteY), i(Nop, Implied), i(Sre, AbsoluteY),
    i(Nop, AbsoluteX), i(Eor, AbsoluteX), i(Lsr, AbsoluteX), i(Sre, AbsoluteX),
    // 0x60
    i(Rts, Implied), i(Adc, IndirectX), i(Jam, Implied), i(Rra, IndirectX),
    i(Nop, ZeroPage), i(Adc, ZeroPage), i(Ror, ZeroPage), i(Rra, ZeroPage),
    i(Pla, Implied), i(Adc, Immediate), i(Ror, Accumulator), i(Arr, Immediate),
    i(Jmp, Indirect), i(Adc, Absolute), i(Ror, Absolute), i(Rra, Absolute),
    // 0x70
    i(Branch(Overflow, true), Relative), i(Adc, IndirectY), i(Jam, Implied), i(Rra, IndirectY),
    i(Nop, ZeroPageX), i(Adc, ZeroPageX), i(Ror, ZeroPageX), i(Rra, ZeroPageX),
    i(Sei, Implied), i(Adc, AbsoluteY), i(Nop, Implied), i(Rra, AbsoluteY),
    i(Nop, AbsoluteX), i(Adc, AbsoluteX), i(Ror, AbsoluteX), i(Rra, AbsoluteX),
    // 0x80
    i(Nop, Immediate), i(Sta, IndirectX), i(Nop, Immediate), i(Sax, IndirectX),
    i(Sty, ZeroPage), i(Sta, ZeroPage), i(Stx, ZeroPage), i(Sax, ZeroPage),
    i(Dey, Implied), i(Nop, Immediate), i(Txa, Implied), i(Ane, Immediate),
    i(Sty, Absolute), i(Sta, Absolute), i(Stx, Absolute), i(Sax, Absolute),
    // 0x90
    i(Branch(Carry, false), Relative), i(Sta, IndirectY), i(Jam, Implied), i(Sha, IndirectY),
    i(Sty, ZeroPageX), i(Sta, ZeroPageX), i(Stx, ZeroPageY), i(Sax, ZeroPageY),
    i(Tya, Implied), i(Sta, AbsoluteY), i(Txs, Implied), i(Tas, AbsoluteY),
    i(Shy, AbsoluteX), i(Sta, AbsoluteX), i(Shx, AbsoluteY), i(Sha, AbsoluteY),
    // 0xA0
    i(Ldy, Immediate), i(Lda, IndirectX), i(Ldx, Immediate), i(Lax, IndirectX),
    i(Ldy, ZeroPage), i(Lda, ZeroPage), i(Ldx, ZeroPage), i(Lax, ZeroPage),
    i(Tay, Implied), i(Lda, Immediate), i(Tax, Implied), i(Lxa, Immediate),
    i(Ldy, Absolute), i(Lda, Absolute), i(Ldx, Absolute), i(Lax, Absolute),
    // 0xB0
    i(Branch(Carry, true), Relative), i(Lda, IndirectY), i(Jam, Implied), i(Lax, IndirectY),
    i(Ldy, ZeroPageX), i(Lda, ZeroPageX), i(Ldx, ZeroPageY), i(Lax, ZeroPageY),
    i(Clv, Implied), i(Lda, AbsoluteY), i(Tsx, Implied), i(Las, AbsoluteY),
    i(Ldy, AbsoluteX), i(Lda, AbsoluteX), i(Ldx, AbsoluteY), i(Lax, AbsoluteY),
    // 0xC0
    i(Cpy, Immediate), i(Cmp, IndirectX), i(Nop, Immediate), i(Dcp, IndirectX),
    i(Cpy, ZeroPage), i(Cmp, ZeroPage), i(Dec, ZeroPage), i(Dcp, ZeroPage),
    i(Iny, Implied), i(Cmp, Immediate), i(Dex, Implied), i(Sbx, Immediate),
    i(Cpy, Absolute), i(Cmp, Absolute), i(Dec, Absolute), i(Dcp, Absolute),
    // 0xD0
    i(Branch(Zero, false), Relative), i(Cmp, IndirectY), i(Jam, Implied), i(Dcp, IndirectY),
    i(Nop, ZeroPageX), i(Cmp, ZeroPageX), i(Dec, ZeroPageX), i(Dcp, ZeroPageX),
    i(Cld, Implied), i(Cmp, AbsoluteY), i(Nop, Implied), i(Dcp, AbsoluteY),
    i(Nop, AbsoluteX), i(Cmp, AbsoluteX), i(Dec, AbsoluteX), i(Dcp, AbsoluteX),
    // 0xE0
    i(Cpx, Immediate), i(Sbc, IndirectX), i(Nop, Immediate), i(Isc, IndirectX),
    i(Cpx, ZeroPage), i(Sbc, ZeroPage), i(Inc, ZeroPage), i(Isc, ZeroPage),
    i(Inx, Implied), i(Sbc, Immediate), i(Nop, Implied), i(Sbc, Immediate),
    i(Cpx, Absolute), i(Sbc, Absolute), i(Inc, Absolute), i(Isc, Absolute),
    // 0xF0
    i(Branch(Zero, true), Relative), i(Sbc, IndirectY), i(Jam, Implied), i(Isc, IndirectY),
    i(Nop, ZeroPageX), i(Sbc, ZeroPageX), i(Inc, ZeroPageX), i(Isc, ZeroPageX),
    i(Sed, Implied), i(Sbc, AbsoluteY), i(Nop, Implied), i(Isc, AbsoluteY),
    i(Nop, AbsoluteX), i(Sbc, AbsoluteX), i(Inc, AbsoluteX), i(Isc, AbsoluteX),
];
