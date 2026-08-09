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
pub mod decode;
pub mod disasm;
pub mod isa;

pub use isa::Mos6502;

use decode::{Access, DECODE, Instr, Mode, Op};

pub mod flags {
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

const STACK_BASE: u16 = 0x0100;
const NMI_VECTOR: u16 = 0xFFFA;
const RESET_VECTOR: u16 = 0xFFFC;
const IRQ_VECTOR: u16 = 0xFFFE;

#[derive(Clone, Copy)]
struct Exec {
    instr: Instr,
    cycle: u8,
    lo: u8,
    hi: u8,
    ptr: u8,
    data: u8,
    addr: u16,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InterruptKind {
    Reset,
    Nmi,
    Irq,
}

#[derive(Clone, Copy)]
enum State {
    Fetch,
    Exec(Exec),
    Interrupt {
        kind: InterruptKind,
        cycle: u8,
        lo: u8,
    },
    Halted,
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

    fn fetch_cycle(&mut self, bus: &mut impl Bus) {
        if self.nmi_pending {
            self.nmi_pending = false;
            self.interrupt_cycle(bus, InterruptKind::Nmi, 1, 0);
            return;
        }
        if self.irq_line && !self.flag(flags::INTERRUPT_DISABLE) {
            self.interrupt_cycle(bus, InterruptKind::Irq, 1, 0);
            return;
        }
        let opcode = bus.read(self.pc);
        if !self.rdy {
            return;
        }
        self.pc = self.pc.wrapping_add(1);
        self.state = State::Exec(Exec {
            instr: DECODE[opcode as usize],
            cycle: 1,
            lo: 0,
            hi: 0,
            ptr: 0,
            data: 0,
            addr: 0,
        });
    }

    fn interrupt_cycle(&mut self, bus: &mut impl Bus, kind: InterruptKind, cycle: u8, lo: u8) {
        use InterruptKind::*;
        let mut lo = lo;
        match cycle {
            1 | 2 => {
                bus.read(self.pc);
                if !self.rdy {
                    return;
                }
            }
            3..=5 => {
                let value = match cycle {
                    3 => (self.pc >> 8) as u8,
                    4 => self.pc as u8,
                    _ => (self.p | flags::UNUSED) & !flags::BREAK,
                };
                // Reset runs the same sequence with the pushes suppressed.
                if kind == Reset {
                    bus.read(STACK_BASE + self.s as u16);
                    if !self.rdy {
                        return;
                    }
                } else {
                    bus.write(STACK_BASE + self.s as u16, value);
                }
                self.s = self.s.wrapping_sub(1);
                if cycle == 5 {
                    self.set_flag(flags::INTERRUPT_DISABLE, true);
                }
            }
            6 | 7 => {
                let vector = match kind {
                    Reset => RESET_VECTOR,
                    Nmi => NMI_VECTOR,
                    Irq => IRQ_VECTOR,
                };
                let value = bus.read(vector + (cycle - 6) as u16);
                if !self.rdy {
                    return;
                }
                if cycle == 6 {
                    lo = value;
                } else {
                    self.pc = u16::from_le_bytes([lo, value]);
                    self.state = State::Fetch;
                    return;
                }
            }
            _ => unreachable!(),
        }
        self.state = State::Interrupt {
            kind,
            cycle: cycle + 1,
            lo,
        };
    }

    fn exec_cycle(&mut self, bus: &mut impl Bus, mut e: Exec) {
        use Mode::*;
        use Op::*;

        macro_rules! read {
            ($addr:expr) => {{
                let value = bus.read($addr);
                if !self.rdy {
                    return;
                }
                value
            }};
        }
        macro_rules! finish {
            () => {{
                self.state = State::Fetch;
                return;
            }};
        }
        macro_rules! next {
            () => {{
                e.cycle += 1;
                self.state = State::Exec(e);
                return;
            }};
        }

        let index = match e.instr.mode {
            ZeroPageX | AbsoluteX => self.x,
            ZeroPageY | AbsoluteY | IndirectY => self.y,
            IndirectX => self.x,
            _ => 0,
        };

        // Bespoke control-flow instructions first.
        match (e.instr.op, e.instr.mode, e.cycle) {
            // JAM: the sequencer wedges; the address bus walks
            // $FFFF/$FFFE/$FFFE then parks on $FFFF until reset.
            (Jam, _, 1) => {
                read!(self.pc);
                next!();
            }
            (Jam, _, 2) => {
                read!(0xFFFF);
                next!();
            }
            (Jam, _, 3) => {
                read!(0xFFFE);
                next!();
            }
            (Jam, _, 4) => {
                read!(0xFFFE);
                self.state = State::Halted;
                return;
            }
            (Branch(flag, expected), _, 1) => {
                e.data = read!(self.pc);
                self.pc = self.pc.wrapping_add(1);
                if self.branch_taken(flag, expected) {
                    next!();
                }
                finish!();
            }
            (Branch(..), _, 2) => {
                read!(self.pc);
                let target = self.pc.wrapping_add(e.data as i8 as u16);
                if target & 0xFF00 == self.pc & 0xFF00 {
                    self.pc = target;
                    finish!();
                }
                e.addr = target;
                self.pc = (self.pc & 0xFF00) | (target & 0x00FF);
                next!();
            }
            (Branch(..), _, 3) => {
                read!(self.pc);
                self.pc = e.addr;
                finish!();
            }
            (Jmp, Absolute, 1) => {
                e.lo = read!(self.pc);
                self.pc = self.pc.wrapping_add(1);
                next!();
            }
            (Jmp, Absolute, 2) => {
                let hi = read!(self.pc);
                self.pc = u16::from_le_bytes([e.lo, hi]);
                finish!();
            }
            (Jmp, Indirect, 1) => {
                e.lo = read!(self.pc);
                self.pc = self.pc.wrapping_add(1);
                next!();
            }
            (Jmp, Indirect, 2) => {
                e.hi = read!(self.pc);
                self.pc = self.pc.wrapping_add(1);
                e.addr = u16::from_le_bytes([e.lo, e.hi]);
                next!();
            }
            (Jmp, Indirect, 3) => {
                e.data = read!(e.addr);
                next!();
            }
            (Jmp, Indirect, 4) => {
                // The NMOS page-wrap bug: the pointer high byte never carries.
                let wrapped = (e.addr & 0xFF00) | (e.addr.wrapping_add(1) & 0x00FF);
                let hi = read!(wrapped);
                self.pc = u16::from_le_bytes([e.data, hi]);
                finish!();
            }
            (Jsr, _, 1) => {
                e.lo = read!(self.pc);
                self.pc = self.pc.wrapping_add(1);
                next!();
            }
            (Jsr, _, 2) => {
                read!(STACK_BASE + self.s as u16);
                next!();
            }
            (Jsr, _, 3) => {
                bus.write(STACK_BASE + self.s as u16, (self.pc >> 8) as u8);
                self.s = self.s.wrapping_sub(1);
                next!();
            }
            (Jsr, _, 4) => {
                bus.write(STACK_BASE + self.s as u16, self.pc as u8);
                self.s = self.s.wrapping_sub(1);
                next!();
            }
            (Jsr, _, 5) => {
                let hi = read!(self.pc);
                self.pc = u16::from_le_bytes([e.lo, hi]);
                finish!();
            }
            (Rts, _, 1) | (Rti, _, 1) => {
                read!(self.pc);
                next!();
            }
            (Rts, _, 2) | (Rti, _, 2) => {
                read!(STACK_BASE + self.s as u16);
                next!();
            }
            (Rts, _, 3) => {
                self.s = self.s.wrapping_add(1);
                e.lo = read!(STACK_BASE + self.s as u16);
                next!();
            }
            (Rts, _, 4) => {
                self.s = self.s.wrapping_add(1);
                let hi = read!(STACK_BASE + self.s as u16);
                self.pc = u16::from_le_bytes([e.lo, hi]);
                next!();
            }
            (Rts, _, 5) => {
                read!(self.pc);
                self.pc = self.pc.wrapping_add(1);
                finish!();
            }
            (Rti, _, 3) => {
                self.s = self.s.wrapping_add(1);
                // B and unused aren't flip-flops: pulls read them as 0/1.
                self.p = (read!(STACK_BASE + self.s as u16) & !flags::BREAK) | flags::UNUSED;
                next!();
            }
            (Rti, _, 4) => {
                self.s = self.s.wrapping_add(1);
                e.lo = read!(STACK_BASE + self.s as u16);
                next!();
            }
            (Rti, _, 5) => {
                self.s = self.s.wrapping_add(1);
                let hi = read!(STACK_BASE + self.s as u16);
                self.pc = u16::from_le_bytes([e.lo, hi]);
                finish!();
            }
            (Brk, _, 1) => {
                read!(self.pc);
                self.pc = self.pc.wrapping_add(1);
                next!();
            }
            (Brk, _, 2) => {
                bus.write(STACK_BASE + self.s as u16, (self.pc >> 8) as u8);
                self.s = self.s.wrapping_sub(1);
                next!();
            }
            (Brk, _, 3) => {
                bus.write(STACK_BASE + self.s as u16, self.pc as u8);
                self.s = self.s.wrapping_sub(1);
                next!();
            }
            (Brk, _, 4) => {
                bus.write(
                    STACK_BASE + self.s as u16,
                    self.p | flags::BREAK | flags::UNUSED,
                );
                self.s = self.s.wrapping_sub(1);
                self.set_flag(flags::INTERRUPT_DISABLE, true);
                next!();
            }
            (Brk, _, 5) => {
                e.lo = read!(IRQ_VECTOR);
                next!();
            }
            (Brk, _, 6) => {
                let hi = read!(IRQ_VECTOR + 1);
                self.pc = u16::from_le_bytes([e.lo, hi]);
                finish!();
            }
            (Pha | Php, _, 1) | (Pla | Plp, _, 1) => {
                read!(self.pc);
                next!();
            }
            (Pha, _, 2) | (Php, _, 2) => {
                let value = match e.instr.op {
                    Pha => self.a,
                    _ => self.p | flags::BREAK | flags::UNUSED,
                };
                bus.write(STACK_BASE + self.s as u16, value);
                self.s = self.s.wrapping_sub(1);
                finish!();
            }
            (Pla | Plp, _, 2) => {
                read!(STACK_BASE + self.s as u16);
                next!();
            }
            (Pla, _, 3) | (Plp, _, 3) => {
                self.s = self.s.wrapping_add(1);
                let value = read!(STACK_BASE + self.s as u16);
                match e.instr.op {
                    Pla => {
                        self.a = value;
                        self.set_zn(value);
                    }
                    // B and unused aren't flip-flops: pulls read them as 0/1.
                    _ => self.p = (value & !flags::BREAK) | flags::UNUSED,
                }
                finish!();
            }
            _ => {}
        }

        // Two-cycle implied and accumulator forms.
        if e.instr.mode == Implied || e.instr.mode == Accumulator {
            read!(self.pc);
            if e.instr.mode == Accumulator {
                self.a = self.apply_rmw(e.instr.op, self.a);
            } else {
                self.apply_implied(e.instr.op);
            }
            finish!();
        }

        // Operand-addressed instructions, sequenced by access class.
        match e.instr.op.access() {
            Access::Read => match (e.instr.mode, e.cycle) {
                (Immediate, 1) => {
                    let value = read!(self.pc);
                    self.pc = self.pc.wrapping_add(1);
                    self.apply_read(e.instr.op, value);
                    finish!();
                }
                (ZeroPage, 1)
                | (ZeroPageX | ZeroPageY, 1)
                | (Absolute | AbsoluteX | AbsoluteY, 1) => {
                    e.lo = read!(self.pc);
                    self.pc = self.pc.wrapping_add(1);
                    next!();
                }
                (ZeroPage, 2) => {
                    let value = read!(e.lo as u16);
                    self.apply_read(e.instr.op, value);
                    finish!();
                }
                (ZeroPageX | ZeroPageY, 2) => {
                    read!(e.lo as u16);
                    e.addr = e.lo.wrapping_add(index) as u16;
                    next!();
                }
                (ZeroPageX | ZeroPageY, 3)
                | (IndirectX, 5)
                | (IndirectY, 5)
                | (AbsoluteX | AbsoluteY, 4) => {
                    let value = read!(e.addr);
                    self.apply_read(e.instr.op, value);
                    finish!();
                }
                (Absolute, 2) => {
                    e.hi = read!(self.pc);
                    self.pc = self.pc.wrapping_add(1);
                    e.addr = u16::from_le_bytes([e.lo, e.hi]);
                    next!();
                }
                (Absolute, 3) => {
                    let value = read!(e.addr);
                    self.apply_read(e.instr.op, value);
                    finish!();
                }
                (AbsoluteX | AbsoluteY, 2) => {
                    e.hi = read!(self.pc);
                    self.pc = self.pc.wrapping_add(1);
                    next!();
                }
                (AbsoluteX | AbsoluteY, 3) => {
                    let sum = e.lo as u16 + index as u16;
                    let unfixed = u16::from_le_bytes([sum as u8, e.hi]);
                    let value = read!(unfixed);
                    if sum < 0x100 {
                        self.apply_read(e.instr.op, value);
                        finish!();
                    }
                    e.addr = unfixed.wrapping_add(0x100);
                    next!();
                }
                (IndirectX | IndirectY, 1) => {
                    e.ptr = read!(self.pc);
                    self.pc = self.pc.wrapping_add(1);
                    next!();
                }
                (IndirectX, 2) => {
                    read!(e.ptr as u16);
                    next!();
                }
                (IndirectX, 3) => {
                    e.lo = read!(e.ptr.wrapping_add(index) as u16);
                    next!();
                }
                (IndirectX, 4) => {
                    e.hi = read!(e.ptr.wrapping_add(index).wrapping_add(1) as u16);
                    e.addr = u16::from_le_bytes([e.lo, e.hi]);
                    next!();
                }
                (IndirectY, 2) => {
                    e.lo = read!(e.ptr as u16);
                    next!();
                }
                (IndirectY, 3) => {
                    e.hi = read!(e.ptr.wrapping_add(1) as u16);
                    next!();
                }
                (IndirectY, 4) => {
                    let sum = e.lo as u16 + index as u16;
                    let unfixed = u16::from_le_bytes([sum as u8, e.hi]);
                    let value = read!(unfixed);
                    if sum < 0x100 {
                        self.apply_read(e.instr.op, value);
                        finish!();
                    }
                    e.addr = unfixed.wrapping_add(0x100);
                    next!();
                }
                _ => unreachable!(),
            },
            Access::Write => match (e.instr.mode, e.cycle) {
                (ZeroPage | ZeroPageX | ZeroPageY | Absolute | AbsoluteX | AbsoluteY, 1) => {
                    e.lo = read!(self.pc);
                    self.pc = self.pc.wrapping_add(1);
                    next!();
                }
                (ZeroPage, 2) => {
                    let value = self.write_value(e.instr.op, 0);
                    bus.write(e.lo as u16, value);
                    finish!();
                }
                (ZeroPageX | ZeroPageY, 2) => {
                    read!(e.lo as u16);
                    e.addr = e.lo.wrapping_add(index) as u16;
                    next!();
                }
                (ZeroPageX | ZeroPageY, 3) | (IndirectX, 5) => {
                    let value = self.write_value(e.instr.op, e.hi);
                    bus.write(e.addr, value);
                    finish!();
                }
                (Absolute | AbsoluteX | AbsoluteY, 2) => {
                    e.hi = read!(self.pc);
                    self.pc = self.pc.wrapping_add(1);
                    e.addr = u16::from_le_bytes([e.lo, e.hi]);
                    next!();
                }
                (Absolute, 3) => {
                    let value = self.write_value(e.instr.op, e.hi);
                    bus.write(e.addr, value);
                    finish!();
                }
                (AbsoluteX | AbsoluteY, 3) => {
                    let sum = e.lo as u16 + index as u16;
                    let unfixed = u16::from_le_bytes([sum as u8, e.hi]);
                    read!(unfixed);
                    e.addr = self.indexed_store_address(e.instr.op, e.hi, sum);
                    next!();
                }
                (AbsoluteX | AbsoluteY, 4) | (IndirectY, 5) => {
                    let value = self.write_value(e.instr.op, e.hi);
                    bus.write(e.addr, value);
                    finish!();
                }
                (IndirectX | IndirectY, 1) => {
                    e.ptr = read!(self.pc);
                    self.pc = self.pc.wrapping_add(1);
                    next!();
                }
                (IndirectX, 2) => {
                    read!(e.ptr as u16);
                    next!();
                }
                (IndirectX, 3) => {
                    e.lo = read!(e.ptr.wrapping_add(index) as u16);
                    next!();
                }
                (IndirectX, 4) => {
                    e.hi = read!(e.ptr.wrapping_add(index).wrapping_add(1) as u16);
                    e.addr = u16::from_le_bytes([e.lo, e.hi]);
                    next!();
                }
                (IndirectY, 2) => {
                    e.lo = read!(e.ptr as u16);
                    next!();
                }
                (IndirectY, 3) => {
                    e.hi = read!(e.ptr.wrapping_add(1) as u16);
                    next!();
                }
                (IndirectY, 4) => {
                    let sum = e.lo as u16 + index as u16;
                    let unfixed = u16::from_le_bytes([sum as u8, e.hi]);
                    read!(unfixed);
                    e.addr = self.indexed_store_address(e.instr.op, e.hi, sum);
                    next!();
                }
                _ => unreachable!(),
            },
            Access::ReadModifyWrite => match (e.instr.mode, e.cycle) {
                (ZeroPage | ZeroPageX | Absolute | AbsoluteX | AbsoluteY, 1) => {
                    e.lo = read!(self.pc);
                    self.pc = self.pc.wrapping_add(1);
                    if e.instr.mode == ZeroPage {
                        e.addr = e.lo as u16;
                    }
                    next!();
                }
                (ZeroPageX, 2) => {
                    read!(e.lo as u16);
                    e.addr = e.lo.wrapping_add(index) as u16;
                    next!();
                }
                (Absolute | AbsoluteX | AbsoluteY, 2) => {
                    e.hi = read!(self.pc);
                    self.pc = self.pc.wrapping_add(1);
                    e.addr = u16::from_le_bytes([e.lo, e.hi]);
                    next!();
                }
                (AbsoluteX | AbsoluteY, 3) => {
                    let sum = e.lo as u16 + index as u16;
                    let unfixed = u16::from_le_bytes([sum as u8, e.hi]);
                    read!(unfixed);
                    e.addr = if sum < 0x100 {
                        unfixed
                    } else {
                        unfixed.wrapping_add(0x100)
                    };
                    next!();
                }
                (ZeroPage, 2)
                | (ZeroPageX, 3)
                | (Absolute, 3)
                | (AbsoluteX | AbsoluteY, 4)
                | (IndirectX, 5)
                | (IndirectY, 5) => {
                    e.data = read!(e.addr);
                    next!();
                }
                (ZeroPage, 3)
                | (ZeroPageX, 4)
                | (Absolute, 4)
                | (AbsoluteX | AbsoluteY, 5)
                | (IndirectX, 6)
                | (IndirectY, 6) => {
                    bus.write(e.addr, e.data);
                    e.data = self.apply_rmw(e.instr.op, e.data);
                    next!();
                }
                (ZeroPage, 4)
                | (ZeroPageX, 5)
                | (Absolute, 5)
                | (AbsoluteX | AbsoluteY, 6)
                | (IndirectX, 7)
                | (IndirectY, 7) => {
                    bus.write(e.addr, e.data);
                    finish!();
                }
                (IndirectX | IndirectY, 1) => {
                    e.ptr = read!(self.pc);
                    self.pc = self.pc.wrapping_add(1);
                    next!();
                }
                (IndirectX, 2) => {
                    read!(e.ptr as u16);
                    next!();
                }
                (IndirectX, 3) => {
                    e.lo = read!(e.ptr.wrapping_add(index) as u16);
                    next!();
                }
                (IndirectX, 4) => {
                    e.hi = read!(e.ptr.wrapping_add(index).wrapping_add(1) as u16);
                    e.addr = u16::from_le_bytes([e.lo, e.hi]);
                    next!();
                }
                (IndirectY, 2) => {
                    e.lo = read!(e.ptr as u16);
                    next!();
                }
                (IndirectY, 3) => {
                    e.hi = read!(e.ptr.wrapping_add(1) as u16);
                    next!();
                }
                (IndirectY, 4) => {
                    let sum = e.lo as u16 + index as u16;
                    let unfixed = u16::from_le_bytes([sum as u8, e.hi]);
                    read!(unfixed);
                    e.addr = if sum < 0x100 {
                        unfixed
                    } else {
                        unfixed.wrapping_add(0x100)
                    };
                    next!();
                }
                _ => unreachable!(),
            },
        }
    }

    /// The value a store-class instruction drives onto the data bus.
    /// `base_hi` is the pre-index high address byte the SH-family ANDs with.
    fn write_value(&mut self, op: Op, base_hi: u8) -> u8 {
        match op {
            Op::Sta => self.a,
            Op::Stx => self.x,
            Op::Sty => self.y,
            Op::Sax => self.a & self.x,
            Op::Sha => self.a & self.x & base_hi.wrapping_add(1),
            Op::Shx => self.x & base_hi.wrapping_add(1),
            Op::Shy => self.y & base_hi.wrapping_add(1),
            Op::Tas => {
                self.s = self.a & self.x;
                self.s & base_hi.wrapping_add(1)
            }
            _ => unreachable!("not a write op: {op:?}"),
        }
    }

    /// Indexed-store target fixup. The SH-family anomaly: on a page
    /// crossing, the driven value replaces the carried high address byte.
    fn indexed_store_address(&mut self, op: Op, base_hi: u8, sum: u16) -> u16 {
        let crossed = sum >= 0x100;
        let hi = match op {
            Op::Sha | Op::Shx | Op::Shy | Op::Tas if crossed => self.write_value(op, base_hi),
            _ if crossed => base_hi.wrapping_add(1),
            _ => base_hi,
        };
        u16::from_le_bytes([sum as u8, hi])
    }
}

impl<B: Bus> missingno_core::ClockedCpu<B> for Cpu {
    fn tick(&mut self, bus: &mut B) {
        Cpu::tick(self, bus);
    }

    fn at_instruction_boundary(&self) -> bool {
        Cpu::at_instruction_boundary(self)
    }

    fn jammed(&self) -> bool {
        Cpu::jammed(self)
    }
}
