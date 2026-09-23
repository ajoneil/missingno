//! The CPU's boundary state in the hardware-named state vocabulary: the fields
//! a board's schema carries for the part, and the capture and restore through
//! a [`StateRecord`] keyed on them.

use missingno_core::state::{FieldDef, FieldType, StateRecord, StateValue};
use missingno_core::system::StateError;

use crate::{CpuState, InterruptMode};

use FieldType::{Bool, U8, U16};

/// The register file as observable fields, then the boundary carries a
/// bit-exact restore also needs.
pub fn state_fields() -> Vec<FieldDef> {
    vec![
        FieldDef::observable("a", U8, "cpu").help("accumulator"),
        FieldDef::observable("f", U8, "cpu").help("flags"),
        FieldDef::observable("b", U8, "cpu"),
        FieldDef::observable("c", U8, "cpu"),
        FieldDef::observable("d", U8, "cpu"),
        FieldDef::observable("e", U8, "cpu"),
        FieldDef::observable("h", U8, "cpu"),
        FieldDef::observable("l", U8, "cpu"),
        FieldDef::observable("a_alt", U8, "cpu").help("A' — the alternate set"),
        FieldDef::observable("f_alt", U8, "cpu").help("F'"),
        FieldDef::observable("b_alt", U8, "cpu").help("B'"),
        FieldDef::observable("c_alt", U8, "cpu").help("C'"),
        FieldDef::observable("d_alt", U8, "cpu").help("D'"),
        FieldDef::observable("e_alt", U8, "cpu").help("E'"),
        FieldDef::observable("h_alt", U8, "cpu").help("H'"),
        FieldDef::observable("l_alt", U8, "cpu").help("L'"),
        FieldDef::observable("ix", U16, "cpu"),
        FieldDef::observable("iy", U16, "cpu"),
        FieldDef::observable("sp", U16, "cpu").help("stack pointer"),
        FieldDef::observable("pc", U16, "cpu").help("program counter"),
        FieldDef::observable("i", U8, "cpu").help("interrupt vector page"),
        FieldDef::observable("r", U8, "cpu").help("memory refresh counter"),
        FieldDef::observable("iff1", Bool, "cpu").help("interrupts enabled"),
        FieldDef::observable("iff2", Bool, "cpu").help("IFF1's copy, which LD A,I reads"),
        FieldDef::observable("im", U8, "cpu").help("interrupt mode 0/1/2"),
        FieldDef::observable("halted", Bool, "cpu").help("HALT is refetching"),
        FieldDef::boundary("wz", U16, "cpu").help("MEMPTR"),
        FieldDef::boundary("q", U8, "cpu").help("F left by the last flag-modifying instruction"),
        FieldDef::boundary("p", Bool, "cpu").help("LD A,I / LD A,R just took PF from IFF2"),
        FieldDef::boundary("ei_pending", Bool, "cpu")
            .help("acceptance is held off for the instruction after EI"),
        FieldDef::boundary("flags_touched", Bool, "cpu")
            .help("the retiring instruction wrote flags, so Q takes F"),
        FieldDef::boundary("nmi_pending", Bool, "cpu").help("an /NMI edge awaits acceptance"),
        FieldDef::boundary("nmi_line", Bool, "cpu").help("/NMI as the board last drove it"),
        FieldDef::boundary("irq_line", Bool, "cpu").help("/INT as the board drives it"),
        FieldDef::boundary("irq_sampled", Bool, "cpu")
            .help("/INT as sampled at the last instruction's final T-state"),
        FieldDef::boundary("address_bus", U16, "cpu")
            .help("the address the pins hold through an internal T-state"),
    ]
}

pub fn write_state(record: &mut StateRecord, cpu: &CpuState) {
    record
        .set("a", cpu.a)
        .set("f", cpu.f)
        .set("b", cpu.b)
        .set("c", cpu.c)
        .set("d", cpu.d)
        .set("e", cpu.e)
        .set("h", cpu.h)
        .set("l", cpu.l)
        .set("a_alt", cpu.a_alt)
        .set("f_alt", cpu.f_alt)
        .set("b_alt", cpu.b_alt)
        .set("c_alt", cpu.c_alt)
        .set("d_alt", cpu.d_alt)
        .set("e_alt", cpu.e_alt)
        .set("h_alt", cpu.h_alt)
        .set("l_alt", cpu.l_alt)
        .set("ix", cpu.ix)
        .set("iy", cpu.iy)
        .set("sp", cpu.sp)
        .set("pc", cpu.pc)
        .set("i", cpu.i)
        .set("r", cpu.r)
        .set("iff1", cpu.iff1)
        .set("iff2", cpu.iff2)
        .set("im", interrupt_mode_number(cpu.interrupt_mode))
        .set("halted", cpu.halted)
        .set("wz", cpu.wz)
        .set("q", cpu.q)
        .set("p", cpu.p)
        .set("ei_pending", cpu.ei_pending)
        .set("flags_touched", cpu.flags_touched)
        .set("nmi_pending", cpu.nmi_pending)
        .set("nmi_line", cpu.nmi_line)
        .set("irq_line", cpu.irq_line)
        .set("irq_sampled", cpu.irq_sampled)
        .set("address_bus", cpu.address_bus);
}

/// The CPU a record describes. A record written before `nmi_line` existed
/// reads it as released.
pub fn parse_state(r: &StateRecord) -> Result<CpuState, StateError> {
    Ok(CpuState {
        a: u8_of(r, "a")?,
        f: u8_of(r, "f")?,
        b: u8_of(r, "b")?,
        c: u8_of(r, "c")?,
        d: u8_of(r, "d")?,
        e: u8_of(r, "e")?,
        h: u8_of(r, "h")?,
        l: u8_of(r, "l")?,
        a_alt: u8_of(r, "a_alt")?,
        f_alt: u8_of(r, "f_alt")?,
        b_alt: u8_of(r, "b_alt")?,
        c_alt: u8_of(r, "c_alt")?,
        d_alt: u8_of(r, "d_alt")?,
        e_alt: u8_of(r, "e_alt")?,
        h_alt: u8_of(r, "h_alt")?,
        l_alt: u8_of(r, "l_alt")?,
        ix: u16_of(r, "ix")?,
        iy: u16_of(r, "iy")?,
        sp: u16_of(r, "sp")?,
        pc: u16_of(r, "pc")?,
        wz: u16_of(r, "wz")?,
        i: u8_of(r, "i")?,
        r: u8_of(r, "r")?,
        iff1: bool_of(r, "iff1")?,
        iff2: bool_of(r, "iff2")?,
        interrupt_mode: interrupt_mode(u8_of(r, "im")?)?,
        halted: bool_of(r, "halted")?,
        ei_pending: bool_of(r, "ei_pending")?,
        q: u8_of(r, "q")?,
        flags_touched: bool_of(r, "flags_touched")?,
        p: bool_of(r, "p")?,
        nmi_pending: bool_of(r, "nmi_pending")?,
        nmi_line: match r.get("nmi_line") {
            None => false,
            Some(_) => bool_of(r, "nmi_line")?,
        },
        irq_line: bool_of(r, "irq_line")?,
        irq_sampled: bool_of(r, "irq_sampled")?,
        address_bus: u16_of(r, "address_bus")?,
    })
}

fn interrupt_mode_number(mode: InterruptMode) -> u8 {
    match mode {
        InterruptMode::Mode0 => 0,
        InterruptMode::Mode1 => 1,
        InterruptMode::Mode2 => 2,
    }
}

fn interrupt_mode(number: u8) -> Result<InterruptMode, StateError> {
    match number {
        0 => Ok(InterruptMode::Mode0),
        1 => Ok(InterruptMode::Mode1),
        2 => Ok(InterruptMode::Mode2),
        _ => Err(StateError::Corrupt),
    }
}

fn u8_of(r: &StateRecord, name: &str) -> Result<u8, StateError> {
    match r.get(name) {
        Some(StateValue::Int(value)) if *value <= u8::MAX as u32 => Ok(*value as u8),
        _ => Err(StateError::Corrupt),
    }
}

fn u16_of(r: &StateRecord, name: &str) -> Result<u16, StateError> {
    match r.get(name) {
        Some(StateValue::Int(value)) if *value <= u16::MAX as u32 => Ok(*value as u16),
        _ => Err(StateError::Corrupt),
    }
}

fn bool_of(r: &StateRecord, name: &str) -> Result<bool, StateError> {
    match r.get(name) {
        Some(StateValue::Bool(value)) => Ok(*value),
        _ => Err(StateError::Corrupt),
    }
}
