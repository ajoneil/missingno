//! The cycle sequencer: where a part-executed instruction stands, and the one
//! bus access each tick performs.
//!
//! Each function below walks one instruction shape a cycle at a time. A cycle
//! issues its single access and then does exactly one of three things: freeze
//! (RDY low, so the next tick re-issues the same access), hand the instruction
//! its next cycle, or retire it.

use crate::decode::{Access, DECODE, Flag, Instr, Mode, Op};
use crate::{Bus, Cpu, flags};

const STACK_BASE: u16 = 0x0100;
const NMI_VECTOR: u16 = 0xFFFA;
const RESET_VECTOR: u16 = 0xFFFC;
const IRQ_VECTOR: u16 = 0xFFFE;

/// The scratch an instruction carries between its cycles: the operand bytes
/// latched so far and the effective address they build.
#[derive(Clone, Copy)]
pub(crate) struct Exec {
    instr: Instr,
    cycle: u8,
    lo: u8,
    hi: u8,
    ptr: u8,
    data: u8,
    addr: u16,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InterruptKind {
    Reset,
    Nmi,
    Irq,
}

#[derive(Clone, Copy)]
pub(crate) enum State {
    Fetch,
    Exec(Exec),
    Interrupt {
        kind: InterruptKind,
        cycle: u8,
        lo: u8,
    },
    Halted,
}

/// One bus read under RDY: a low line freezes the cycle, leaving the sequencer
/// untouched so the next tick re-issues the same access.
macro_rules! read {
    ($cpu:ident, $bus:ident, $address:expr) => {{
        let value = $bus.read($address);
        if !$cpu.rdy {
            return;
        }
        value
    }};
}

/// Retire the instruction — the next tick fetches an opcode.
macro_rules! finish {
    ($cpu:ident) => {{
        $cpu.state = State::Fetch;
        return;
    }};
}

/// Hand the instruction its next cycle.
macro_rules! next {
    ($cpu:ident, $e:ident) => {{
        $e.cycle += 1;
        $cpu.state = State::Exec($e);
        return;
    }};
}

impl Cpu {
    pub(crate) fn fetch_cycle(&mut self, bus: &mut impl Bus) {
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

    pub(crate) fn interrupt_cycle(
        &mut self,
        bus: &mut impl Bus,
        kind: InterruptKind,
        cycle: u8,
        lo: u8,
    ) {
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

    /// Route the cycle to the sequence its opcode walks: the bespoke ones
    /// first, then the two-cycle register forms, then the operand-addressed
    /// instructions grouped by what they do with the address they resolve.
    pub(crate) fn exec_cycle(&mut self, bus: &mut impl Bus, e: Exec) {
        match e.instr.op {
            Op::Jam => self.jam_cycle(bus, e),
            Op::Branch(flag, expected) => self.branch_cycle(bus, e, flag, expected),
            Op::Jmp => self.jump_cycle(bus, e),
            Op::Jsr => self.subroutine_call_cycle(bus, e),
            Op::Rts | Op::Rti => self.return_cycle(bus, e),
            Op::Brk => self.break_cycle(bus, e),
            Op::Pha | Op::Php | Op::Pla | Op::Plp => self.stack_cycle(bus, e),
            _ if matches!(e.instr.mode, Mode::Implied | Mode::Accumulator) => {
                self.register_cycle(bus, e)
            }
            _ => match e.instr.op.access() {
                Access::Read => self.read_operand_cycle(bus, e),
                Access::Write => self.write_operand_cycle(bus, e),
                Access::ReadModifyWrite => self.modify_operand_cycle(bus, e),
            },
        }
    }

    /// The register an indexed mode adds to the base address it reads.
    fn index_register(&self, mode: Mode) -> u8 {
        match mode {
            Mode::ZeroPageX | Mode::AbsoluteX | Mode::IndirectX => self.x,
            Mode::ZeroPageY | Mode::AbsoluteY | Mode::IndirectY => self.y,
            _ => 0,
        }
    }
}

/// The bespoke sequences: control flow, the stack instructions, and the JAM
/// wedge. Each walks cycles of its own rather than an addressing mode's.
impl Cpu {
    /// JAM: the sequencer wedges; the address bus walks $FFFF/$FFFE/$FFFE then
    /// parks on $FFFF until reset.
    fn jam_cycle(&mut self, bus: &mut impl Bus, mut e: Exec) {
        match e.cycle {
            1 => {
                read!(self, bus, self.pc);
                next!(self, e);
            }
            2 => {
                read!(self, bus, 0xFFFF);
                next!(self, e);
            }
            3 => {
                read!(self, bus, 0xFFFE);
                next!(self, e);
            }
            4 => {
                read!(self, bus, 0xFFFE);
                self.state = State::Halted;
            }
            _ => unreachable!(),
        }
    }

    fn branch_cycle(&mut self, bus: &mut impl Bus, mut e: Exec, flag: Flag, expected: bool) {
        match e.cycle {
            1 => {
                e.data = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                if self.branch_taken(flag, expected) {
                    next!(self, e);
                }
                finish!(self);
            }
            2 => {
                read!(self, bus, self.pc);
                let target = self.pc.wrapping_add(e.data as i8 as u16);
                if target & 0xFF00 == self.pc & 0xFF00 {
                    self.pc = target;
                    finish!(self);
                }
                e.addr = target;
                self.pc = (self.pc & 0xFF00) | (target & 0x00FF);
                next!(self, e);
            }
            3 => {
                read!(self, bus, self.pc);
                self.pc = e.addr;
                finish!(self);
            }
            _ => unreachable!(),
        }
    }

    fn jump_cycle(&mut self, bus: &mut impl Bus, mut e: Exec) {
        use Mode::*;
        match (e.instr.mode, e.cycle) {
            (Absolute | Indirect, 1) => {
                e.lo = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                next!(self, e);
            }
            (Absolute, 2) => {
                let hi = read!(self, bus, self.pc);
                self.pc = u16::from_le_bytes([e.lo, hi]);
                finish!(self);
            }
            (Indirect, 2) => {
                e.hi = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                e.addr = u16::from_le_bytes([e.lo, e.hi]);
                next!(self, e);
            }
            (Indirect, 3) => {
                e.data = read!(self, bus, e.addr);
                next!(self, e);
            }
            (Indirect, 4) => {
                // The NMOS page-wrap bug: the pointer high byte never carries.
                let wrapped = (e.addr & 0xFF00) | (e.addr.wrapping_add(1) & 0x00FF);
                let hi = read!(self, bus, wrapped);
                self.pc = u16::from_le_bytes([e.data, hi]);
                finish!(self);
            }
            _ => unreachable!(),
        }
    }

    fn subroutine_call_cycle(&mut self, bus: &mut impl Bus, mut e: Exec) {
        match e.cycle {
            1 => {
                e.lo = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                next!(self, e);
            }
            2 => {
                read!(self, bus, STACK_BASE + self.s as u16);
                next!(self, e);
            }
            3 => {
                bus.write(STACK_BASE + self.s as u16, (self.pc >> 8) as u8);
                self.s = self.s.wrapping_sub(1);
                next!(self, e);
            }
            4 => {
                bus.write(STACK_BASE + self.s as u16, self.pc as u8);
                self.s = self.s.wrapping_sub(1);
                next!(self, e);
            }
            5 => {
                let hi = read!(self, bus, self.pc);
                self.pc = u16::from_le_bytes([e.lo, hi]);
                finish!(self);
            }
            _ => unreachable!(),
        }
    }

    /// RTS and RTI, which share their two lead-in cycles before pulling
    /// different numbers of bytes.
    fn return_cycle(&mut self, bus: &mut impl Bus, mut e: Exec) {
        use Op::*;
        match (e.instr.op, e.cycle) {
            (Rts | Rti, 1) => {
                read!(self, bus, self.pc);
                next!(self, e);
            }
            (Rts | Rti, 2) => {
                read!(self, bus, STACK_BASE + self.s as u16);
                next!(self, e);
            }
            (Rts, 3) => {
                self.s = self.s.wrapping_add(1);
                e.lo = read!(self, bus, STACK_BASE + self.s as u16);
                next!(self, e);
            }
            (Rts, 4) => {
                self.s = self.s.wrapping_add(1);
                let hi = read!(self, bus, STACK_BASE + self.s as u16);
                self.pc = u16::from_le_bytes([e.lo, hi]);
                next!(self, e);
            }
            (Rts, 5) => {
                read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                finish!(self);
            }
            (Rti, 3) => {
                self.s = self.s.wrapping_add(1);
                // B and unused aren't flip-flops: pulls read them as 0/1.
                self.p =
                    (read!(self, bus, STACK_BASE + self.s as u16) & !flags::BREAK) | flags::UNUSED;
                next!(self, e);
            }
            (Rti, 4) => {
                self.s = self.s.wrapping_add(1);
                e.lo = read!(self, bus, STACK_BASE + self.s as u16);
                next!(self, e);
            }
            (Rti, 5) => {
                self.s = self.s.wrapping_add(1);
                let hi = read!(self, bus, STACK_BASE + self.s as u16);
                self.pc = u16::from_le_bytes([e.lo, hi]);
                finish!(self);
            }
            _ => unreachable!(),
        }
    }

    fn break_cycle(&mut self, bus: &mut impl Bus, mut e: Exec) {
        match e.cycle {
            1 => {
                read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                next!(self, e);
            }
            2 => {
                bus.write(STACK_BASE + self.s as u16, (self.pc >> 8) as u8);
                self.s = self.s.wrapping_sub(1);
                next!(self, e);
            }
            3 => {
                bus.write(STACK_BASE + self.s as u16, self.pc as u8);
                self.s = self.s.wrapping_sub(1);
                next!(self, e);
            }
            4 => {
                bus.write(
                    STACK_BASE + self.s as u16,
                    self.p | flags::BREAK | flags::UNUSED,
                );
                self.s = self.s.wrapping_sub(1);
                self.set_flag(flags::INTERRUPT_DISABLE, true);
                next!(self, e);
            }
            5 => {
                e.lo = read!(self, bus, IRQ_VECTOR);
                next!(self, e);
            }
            6 => {
                let hi = read!(self, bus, IRQ_VECTOR + 1);
                self.pc = u16::from_le_bytes([e.lo, hi]);
                finish!(self);
            }
            _ => unreachable!(),
        }
    }

    /// The pushes and pulls, whose second cycle is the stack access the pushes
    /// complete on and the pulls only pre-decrement for.
    fn stack_cycle(&mut self, bus: &mut impl Bus, mut e: Exec) {
        use Op::*;
        match (e.instr.op, e.cycle) {
            (Pha | Php | Pla | Plp, 1) => {
                read!(self, bus, self.pc);
                next!(self, e);
            }
            (Pha | Php, 2) => {
                let value = match e.instr.op {
                    Pha => self.a,
                    _ => self.p | flags::BREAK | flags::UNUSED,
                };
                bus.write(STACK_BASE + self.s as u16, value);
                self.s = self.s.wrapping_sub(1);
                finish!(self);
            }
            (Pla | Plp, 2) => {
                read!(self, bus, STACK_BASE + self.s as u16);
                next!(self, e);
            }
            (Pla | Plp, 3) => {
                self.s = self.s.wrapping_add(1);
                let value = read!(self, bus, STACK_BASE + self.s as u16);
                match e.instr.op {
                    Pla => {
                        self.a = value;
                        self.set_zn(value);
                    }
                    // B and unused aren't flip-flops: pulls read them as 0/1.
                    _ => self.p = (value & !flags::BREAK) | flags::UNUSED,
                }
                finish!(self);
            }
            _ => unreachable!(),
        }
    }

    /// Two-cycle implied and accumulator forms: the dummy operand read, then
    /// the operation on a register.
    fn register_cycle(&mut self, bus: &mut impl Bus, e: Exec) {
        read!(self, bus, self.pc);
        if e.instr.mode == Mode::Accumulator {
            self.a = self.apply_rmw(e.instr.op, self.a);
        } else {
            self.apply_implied(e.instr.op);
        }
        finish!(self);
    }
}

/// Operand-addressed instructions, sequenced by access class: the addressing
/// mode picks the cycles that resolve the address, the access class picks what
/// the instruction does once it lands.
impl Cpu {
    fn read_operand_cycle(&mut self, bus: &mut impl Bus, mut e: Exec) {
        use Mode::*;
        let index = self.index_register(e.instr.mode);
        match (e.instr.mode, e.cycle) {
            (Immediate, 1) => {
                let value = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                self.apply_read(e.instr.op, value);
                finish!(self);
            }
            (ZeroPage, 1) | (ZeroPageX | ZeroPageY, 1) | (Absolute | AbsoluteX | AbsoluteY, 1) => {
                e.lo = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                next!(self, e);
            }
            (ZeroPage, 2) => {
                let value = read!(self, bus, e.lo as u16);
                self.apply_read(e.instr.op, value);
                finish!(self);
            }
            (ZeroPageX | ZeroPageY, 2) => {
                read!(self, bus, e.lo as u16);
                e.addr = e.lo.wrapping_add(index) as u16;
                next!(self, e);
            }
            (ZeroPageX | ZeroPageY, 3)
            | (IndirectX, 5)
            | (IndirectY, 5)
            | (AbsoluteX | AbsoluteY, 4) => {
                let value = read!(self, bus, e.addr);
                self.apply_read(e.instr.op, value);
                finish!(self);
            }
            (Absolute, 2) => {
                e.hi = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                e.addr = u16::from_le_bytes([e.lo, e.hi]);
                next!(self, e);
            }
            (Absolute, 3) => {
                let value = read!(self, bus, e.addr);
                self.apply_read(e.instr.op, value);
                finish!(self);
            }
            (AbsoluteX | AbsoluteY, 2) => {
                e.hi = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                next!(self, e);
            }
            (AbsoluteX | AbsoluteY, 3) => {
                let sum = e.lo as u16 + index as u16;
                let unfixed = u16::from_le_bytes([sum as u8, e.hi]);
                let value = read!(self, bus, unfixed);
                if sum < 0x100 {
                    self.apply_read(e.instr.op, value);
                    finish!(self);
                }
                e.addr = unfixed.wrapping_add(0x100);
                next!(self, e);
            }
            (IndirectX | IndirectY, 1) => {
                e.ptr = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                next!(self, e);
            }
            (IndirectX, 2) => {
                read!(self, bus, e.ptr as u16);
                next!(self, e);
            }
            (IndirectX, 3) => {
                e.lo = read!(self, bus, e.ptr.wrapping_add(index) as u16);
                next!(self, e);
            }
            (IndirectX, 4) => {
                e.hi = read!(self, bus, e.ptr.wrapping_add(index).wrapping_add(1) as u16);
                e.addr = u16::from_le_bytes([e.lo, e.hi]);
                next!(self, e);
            }
            (IndirectY, 2) => {
                e.lo = read!(self, bus, e.ptr as u16);
                next!(self, e);
            }
            (IndirectY, 3) => {
                e.hi = read!(self, bus, e.ptr.wrapping_add(1) as u16);
                next!(self, e);
            }
            (IndirectY, 4) => {
                let sum = e.lo as u16 + index as u16;
                let unfixed = u16::from_le_bytes([sum as u8, e.hi]);
                let value = read!(self, bus, unfixed);
                if sum < 0x100 {
                    self.apply_read(e.instr.op, value);
                    finish!(self);
                }
                e.addr = unfixed.wrapping_add(0x100);
                next!(self, e);
            }
            _ => unreachable!(),
        }
    }

    fn write_operand_cycle(&mut self, bus: &mut impl Bus, mut e: Exec) {
        use Mode::*;
        let index = self.index_register(e.instr.mode);
        match (e.instr.mode, e.cycle) {
            (ZeroPage | ZeroPageX | ZeroPageY | Absolute | AbsoluteX | AbsoluteY, 1) => {
                e.lo = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                next!(self, e);
            }
            (ZeroPage, 2) => {
                let value = self.write_value(e.instr.op, 0);
                bus.write(e.lo as u16, value);
                finish!(self);
            }
            (ZeroPageX | ZeroPageY, 2) => {
                read!(self, bus, e.lo as u16);
                e.addr = e.lo.wrapping_add(index) as u16;
                next!(self, e);
            }
            (ZeroPageX | ZeroPageY, 3) | (IndirectX, 5) => {
                let value = self.write_value(e.instr.op, e.hi);
                bus.write(e.addr, value);
                finish!(self);
            }
            (Absolute | AbsoluteX | AbsoluteY, 2) => {
                e.hi = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                e.addr = u16::from_le_bytes([e.lo, e.hi]);
                next!(self, e);
            }
            (Absolute, 3) => {
                let value = self.write_value(e.instr.op, e.hi);
                bus.write(e.addr, value);
                finish!(self);
            }
            (AbsoluteX | AbsoluteY, 3) => {
                let sum = e.lo as u16 + index as u16;
                let unfixed = u16::from_le_bytes([sum as u8, e.hi]);
                read!(self, bus, unfixed);
                e.addr = self.indexed_store_address(e.instr.op, e.hi, sum);
                next!(self, e);
            }
            (AbsoluteX | AbsoluteY, 4) | (IndirectY, 5) => {
                let value = self.write_value(e.instr.op, e.hi);
                bus.write(e.addr, value);
                finish!(self);
            }
            (IndirectX | IndirectY, 1) => {
                e.ptr = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                next!(self, e);
            }
            (IndirectX, 2) => {
                read!(self, bus, e.ptr as u16);
                next!(self, e);
            }
            (IndirectX, 3) => {
                e.lo = read!(self, bus, e.ptr.wrapping_add(index) as u16);
                next!(self, e);
            }
            (IndirectX, 4) => {
                e.hi = read!(self, bus, e.ptr.wrapping_add(index).wrapping_add(1) as u16);
                e.addr = u16::from_le_bytes([e.lo, e.hi]);
                next!(self, e);
            }
            (IndirectY, 2) => {
                e.lo = read!(self, bus, e.ptr as u16);
                next!(self, e);
            }
            (IndirectY, 3) => {
                e.hi = read!(self, bus, e.ptr.wrapping_add(1) as u16);
                next!(self, e);
            }
            (IndirectY, 4) => {
                let sum = e.lo as u16 + index as u16;
                let unfixed = u16::from_le_bytes([sum as u8, e.hi]);
                read!(self, bus, unfixed);
                e.addr = self.indexed_store_address(e.instr.op, e.hi, sum);
                next!(self, e);
            }
            _ => unreachable!(),
        }
    }

    fn modify_operand_cycle(&mut self, bus: &mut impl Bus, mut e: Exec) {
        use Mode::*;
        let index = self.index_register(e.instr.mode);
        match (e.instr.mode, e.cycle) {
            (ZeroPage | ZeroPageX | Absolute | AbsoluteX | AbsoluteY, 1) => {
                e.lo = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                if e.instr.mode == ZeroPage {
                    e.addr = e.lo as u16;
                }
                next!(self, e);
            }
            (ZeroPageX, 2) => {
                read!(self, bus, e.lo as u16);
                e.addr = e.lo.wrapping_add(index) as u16;
                next!(self, e);
            }
            (Absolute | AbsoluteX | AbsoluteY, 2) => {
                e.hi = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                e.addr = u16::from_le_bytes([e.lo, e.hi]);
                next!(self, e);
            }
            (AbsoluteX | AbsoluteY, 3) => {
                let sum = e.lo as u16 + index as u16;
                let unfixed = u16::from_le_bytes([sum as u8, e.hi]);
                read!(self, bus, unfixed);
                e.addr = if sum < 0x100 {
                    unfixed
                } else {
                    unfixed.wrapping_add(0x100)
                };
                next!(self, e);
            }
            (ZeroPage, 2)
            | (ZeroPageX, 3)
            | (Absolute, 3)
            | (AbsoluteX | AbsoluteY, 4)
            | (IndirectX, 5)
            | (IndirectY, 5) => {
                e.data = read!(self, bus, e.addr);
                next!(self, e);
            }
            (ZeroPage, 3)
            | (ZeroPageX, 4)
            | (Absolute, 4)
            | (AbsoluteX | AbsoluteY, 5)
            | (IndirectX, 6)
            | (IndirectY, 6) => {
                bus.write(e.addr, e.data);
                e.data = self.apply_rmw(e.instr.op, e.data);
                next!(self, e);
            }
            (ZeroPage, 4)
            | (ZeroPageX, 5)
            | (Absolute, 5)
            | (AbsoluteX | AbsoluteY, 6)
            | (IndirectX, 7)
            | (IndirectY, 7) => {
                bus.write(e.addr, e.data);
                finish!(self);
            }
            (IndirectX | IndirectY, 1) => {
                e.ptr = read!(self, bus, self.pc);
                self.pc = self.pc.wrapping_add(1);
                next!(self, e);
            }
            (IndirectX, 2) => {
                read!(self, bus, e.ptr as u16);
                next!(self, e);
            }
            (IndirectX, 3) => {
                e.lo = read!(self, bus, e.ptr.wrapping_add(index) as u16);
                next!(self, e);
            }
            (IndirectX, 4) => {
                e.hi = read!(self, bus, e.ptr.wrapping_add(index).wrapping_add(1) as u16);
                e.addr = u16::from_le_bytes([e.lo, e.hi]);
                next!(self, e);
            }
            (IndirectY, 2) => {
                e.lo = read!(self, bus, e.ptr as u16);
                next!(self, e);
            }
            (IndirectY, 3) => {
                e.hi = read!(self, bus, e.ptr.wrapping_add(1) as u16);
                next!(self, e);
            }
            (IndirectY, 4) => {
                let sum = e.lo as u16 + index as u16;
                let unfixed = u16::from_le_bytes([sum as u8, e.hi]);
                read!(self, bus, unfixed);
                e.addr = if sum < 0x100 {
                    unfixed
                } else {
                    unfixed.wrapping_add(0x100)
                };
                next!(self, e);
            }
            _ => unreachable!(),
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
