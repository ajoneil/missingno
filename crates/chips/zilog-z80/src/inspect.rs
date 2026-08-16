//! The register file as a debugger shows it: one inspection group, built the
//! same way for every board the part sits on.

use missingno_core::inspect::{Register, RegisterGroup, RegisterPurpose, ValueStyle};

use crate::Cpu;

/// The programmer-visible main set — what a register pane names. The alternate
/// set, the interrupt latches and MEMPTR are boundary state, not this view.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct RegisterFile {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub ix: u16,
    pub iy: u16,
    pub sp: u16,
    pub pc: u16,
}

impl From<&Cpu> for RegisterFile {
    fn from(cpu: &Cpu) -> Self {
        RegisterFile {
            a: cpu.a,
            f: cpu.f,
            b: cpu.b,
            c: cpu.c,
            d: cpu.d,
            e: cpu.e,
            h: cpu.h,
            l: cpu.l,
            ix: cpu.ix,
            iy: cpu.iy,
            sp: cpu.sp,
            pc: cpu.pc,
        }
    }
}

pub fn register_groups(registers: &RegisterFile) -> Vec<RegisterGroup> {
    use RegisterPurpose::{PairHigh, PairLow, ProgramCounter, StackPointer};

    let hex = |name, value: u32, bits| Register {
        name,
        value,
        bits,
        style: ValueStyle::Hex,
        help: None,
        purpose: None,
        active: None,
    };
    vec![RegisterGroup {
        name: "cpu",
        registers: vec![
            hex("a", registers.a as u32, 8)
                .help("accumulator")
                .purpose(PairHigh("af")),
            Register {
                name: "f",
                value: registers.f as u32,
                bits: 8,
                style: ValueStyle::Flags(crate::flags::NAMES),
                help: Some("flags register"),
                purpose: Some(PairLow("af")),
                active: None,
            },
            hex("b", registers.b as u32, 8)
                .help("general register B (high byte of BC)")
                .purpose(PairHigh("bc")),
            hex("c", registers.c as u32, 8)
                .help("general register C (low byte of BC)")
                .purpose(PairLow("bc")),
            hex("d", registers.d as u32, 8)
                .help("general register D (high byte of DE)")
                .purpose(PairHigh("de")),
            hex("e", registers.e as u32, 8)
                .help("general register E (low byte of DE)")
                .purpose(PairLow("de")),
            hex("h", registers.h as u32, 8)
                .help("general register H (high byte of HL)")
                .purpose(PairHigh("hl")),
            hex("l", registers.l as u32, 8)
                .help("general register L (low byte of HL)")
                .purpose(PairLow("hl")),
            hex("ix", registers.ix as u32, 16).help("index register IX"),
            hex("iy", registers.iy as u32, 16).help("index register IY"),
            hex("sp", registers.sp as u32, 16)
                .help("stack pointer")
                .purpose(StackPointer),
            hex("pc", registers.pc as u32, 16)
                .help("program counter")
                .purpose(ProgramCounter),
        ],
    }]
}
