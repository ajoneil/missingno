//! NMOS 6502-family core, stepped one bus cycle at a time.
//!
//! Every cycle performs exactly one bus access. Addresses are computed
//! before any state mutation, so a cycle frozen by RDY re-issues the same
//! access on the next call — the hardware's freeze semantics. RDY is
//! honoured on read cycles only; writes always complete.
//!
//! The 6507 packaging differences (13 address lines, no IRQ/NMI pins) live
//! in the console, not here: the console masks the bus and never asserts
//! the interrupt lines. Interrupt dispatch is implemented, but its
//! cycle-level poll points are unverified against an oracle — no VCS
//! software can observe them.

mod apply;
mod decode;
pub mod disasm;
pub mod isa;
mod sequencer;

pub use isa::{Mos6502, step_over_target};

use sequencer::{InterruptKind, State};

pub(crate) mod flags {
    pub const CARRY: u8 = 0x01;
    pub const ZERO: u8 = 0x02;
    pub const INTERRUPT_DISABLE: u8 = 0x04;
    pub const DECIMAL: u8 = 0x08;
    pub const BREAK: u8 = 0x10;
    pub const UNUSED: u8 = 0x20;
    pub const OVERFLOW: u8 = 0x40;
    pub const NEGATIVE: u8 = 0x80;
}

pub trait Bus {
    fn read(&mut self, address: u16) -> u8;
    fn write(&mut self, address: u16, data: u8);
}

pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub s: u8,
    pub p: u8,
    pub pc: u16,
    /// The RDY pin: while low, read cycles freeze (writes complete).
    pub rdy: bool,
    /// The 2A03 carries this core with the decimal-correction circuitry
    /// disconnected: the D flag still sets, but arithmetic stays binary.
    pub(crate) decimal_enabled: bool,
    nmi_pending: bool,
    irq_line: bool,
    state: State,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
            a: 0,
            x: 0,
            y: 0,
            s: 0xFD,
            p: flags::UNUSED | flags::INTERRUPT_DISABLE,
            pc: 0,
            rdy: true,
            decimal_enabled: true,
            nmi_pending: false,
            irq_line: false,
            state: State::Fetch,
        }
    }

    /// A 2A03-style core: the decimal flag exists but never corrects.
    pub fn new_without_decimal() -> Self {
        Cpu {
            decimal_enabled: false,
            ..Cpu::new()
        }
    }

    /// Begin the 7-cycle reset sequence (stack pushes become reads).
    pub fn reset(&mut self) {
        self.state = State::Interrupt {
            kind: InterruptKind::Reset,
            cycle: 1,
            lo: 0,
        };
    }

    pub fn trigger_nmi(&mut self) {
        self.nmi_pending = true;
    }

    pub fn set_irq(&mut self, asserted: bool) {
        self.irq_line = asserted;
    }

    /// True between instructions — the debugger's stepping boundary.
    pub fn at_instruction_boundary(&self) -> bool {
        matches!(self.state, State::Fetch)
    }

    /// True after a JAM opcode: only reset recovers.
    pub fn jammed(&self) -> bool {
        matches!(self.state, State::Halted)
    }

    /// Reseat the register file and place the core at an instruction boundary —
    /// the sequencer at `Fetch`, or `Halted` when the save was taken on a JAM.
    /// A save is taken only between instructions, so there is no mid-instruction
    /// `Exec`/`Interrupt` micro-state to reconstruct; RDY is re-driven by the bus.
    #[allow(clippy::too_many_arguments)]
    pub fn restore_boundary(&mut self, a: u8, x: u8, y: u8, s: u8, p: u8, pc: u16, halted: bool) {
        self.a = a;
        self.x = x;
        self.y = y;
        self.s = s;
        self.p = p;
        self.pc = pc;
        self.rdy = true;
        self.state = if halted { State::Halted } else { State::Fetch };
    }

    pub fn tick(&mut self, bus: &mut impl Bus) {
        match self.state {
            State::Fetch => self.fetch_cycle(bus),
            State::Exec(exec) => self.exec_cycle(bus, exec),
            State::Interrupt { kind, cycle, lo } => self.interrupt_cycle(bus, kind, cycle, lo),
            State::Halted => {
                bus.read(0xFFFF);
            }
        }
    }
}
