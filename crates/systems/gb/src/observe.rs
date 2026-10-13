//! Watching a console run without changing it: a shadow call stack, and the
//! instructions and accesses at the addresses a client asks for, as [`Event`]s.
//! A call closes when SP rises above its return address's slot, however it does.

use std::collections::BTreeSet;

use crate::call::Registers;
use crate::cpu::instructions::{calls_subroutine, instruction_length};
use crate::{Console, Model};

const HALT: u8 = 0x76;
/// Deeper than this, the outermost calls are forgotten (code that calls and
/// never returns, as a crash loops through RST $38).
pub const CALL_STACK_DEPTH: usize = 256;

/// A set of the 65,536 addresses, as a bitmap.
#[derive(Clone)]
pub struct AddressSet {
    bits: Box<[u64; 1024]>,
    empty: bool,
}

impl Default for AddressSet {
    fn default() -> Self {
        AddressSet {
            bits: Box::new([0; 1024]),
            empty: true,
        }
    }
}

impl AddressSet {
    /// The addresses `lo..hi`.
    pub fn insert_range(&mut self, lo: u16, hi: u32) {
        for address in u32::from(lo)..hi.min(0x10000) {
            self.insert(address as u16);
        }
    }

    pub fn insert(&mut self, address: u16) {
        self.bits[usize::from(address >> 6)] |= 1 << (address & 63);
        self.empty = false;
    }

    #[inline]
    pub fn contains(&self, address: u16) -> bool {
        !self.empty && self.bits[usize::from(address >> 6)] & (1 << (address & 63)) != 0
    }

    pub fn is_empty(&self) -> bool {
        self.empty
    }
}

/// What an [`Observer`] reports besides the calls.
#[derive(Clone, Default)]
pub struct Filters {
    /// Each instruction executed at these addresses.
    pub exec: AddressSet,
    /// The CPU's state before each instruction at these addresses.
    pub registers: AddressSet,
    /// Each CPU write to these addresses.
    pub write: AddressSet,
    /// Each CPU read of these addresses, but an instruction's fetch of its own
    /// bytes.
    pub read: AddressSet,
    /// Routine entries, by ROM bank (0 outside the switchable bank) and
    /// address.
    pub entries: BTreeSet<(u16, u16)>,
}

/// How an open call was opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    /// By the CALL or RST at `site`.
    Call { site: u16 },
    /// By an interrupt's dispatch, taken with the CPU at `site`.
    Interrupt { site: u16 },
    /// By coming to one of the [`Filters::entries`] other than by a call; it
    /// closes when SP rises above where it stood, or at the next entry come to
    /// with SP there.
    Entry,
}

/// A call on the shadow stack: the routine's entry, the ROM bank mapped there
/// as it was entered, and the stack slot that closes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenCall {
    pub opened: Opened,
    pub pc: u16,
    pub bank: u16,
    /// Where its return address was pushed; for an entry, where the stack
    /// stood.
    pub slot: u16,
}

/// What the CPU was doing in the step an access came in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doing {
    Instruction,
    /// Dispatching an interrupt (pushing the return address).
    Dispatch,
    /// Halted, or waking from a halt.
    Halt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessKind {
    Read,
    Write,
}

/// A CPU access to a watched address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    pub kind: AccessKind,
    pub address: u16,
    pub value: u8,
    /// The master edge it was committed on.
    pub edge: u64,
    /// The instruction whose step made it, and the edge that step began on.
    pub pc: u16,
    pub began: u64,
    pub doing: Doing,
}

/// The CPU's state before an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuState {
    pub pc: u16,
    pub registers: Registers,
    pub sp: u16,
    pub ime: bool,
    /// The instruction's first three bytes.
    pub code: [u8; 3],
}

/// Something the observer saw, at a master edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A call opened, the innermost now. A call or an interrupt opens at the
    /// edge its instruction or dispatch began on; an entry when it is come to.
    Opened {
        call: OpenCall,
        edge: u64,
    },
    /// The innermost call closed.
    Closed {
        edge: u64,
    },
    /// The stack was [`CALL_STACK_DEPTH`] deep: the outermost was forgotten.
    Forgot,
    /// An instruction in [`Filters::exec`] ran, in the ROM bank mapped there.
    Executed {
        pc: u16,
        bank: u16,
        edge: u64,
    },
    /// The state before an instruction in [`Filters::registers`].
    State {
        state: CpuState,
        edge: u64,
    },
    Access(Access),
    /// A HALT halted the CPU.
    Halted {
        pc: u16,
        bank: u16,
        edge: u64,
    },
    /// An interrupt's dispatch began, the opcode fetched for the next
    /// instruction left unrun. `after` is the edge the instruction before it
    /// began on, where that one's state was reported.
    Dispatching {
        edge: u64,
        after: Option<u64>,
    },
}

/// A shadow call stack, and the events of what [`Filters`] watches.
pub struct Observer {
    filters: Filters,
    entry_at: AddressSet,
    stack: Vec<OpenCall>,
    /// None for an observer that keeps only the call stack.
    events: Option<Vec<Event>>,
    /// The step under way: its instruction, the edge it began on, and what
    /// it read and wrote that is watched.
    step: Option<(u16, u64)>,
    accesses: Vec<(AccessKind, u16, u8, u64)>,
    /// Where the interrupt dispatch under way began.
    dispatch_from: Option<u64>,
    /// The state before the step under way, where it is watched.
    state: Option<CpuState>,
    /// The edge the last instruction began on, where its state was reported.
    stated: Option<u64>,
}

/// The CPU between instructions, before a step.
pub(crate) struct Before {
    pc: u16,
    sp: u16,
    halted: bool,
    dispatching: bool,
    edge: u64,
}

/// The addresses of `filters`' entries, in any bank.
fn entry_addresses(filters: &Filters) -> AddressSet {
    let mut at = AddressSet::default();
    for (_, pc) in &filters.entries {
        at.insert(*pc);
    }
    at
}

impl Observer {
    pub fn new(filters: Filters) -> Observer {
        Observer {
            entry_at: entry_addresses(&filters),
            filters,
            stack: Vec::new(),
            events: Some(Vec::new()),
            step: None,
            accesses: Vec::new(),
            dispatch_from: None,
            state: None,
            stated: None,
        }
    }

    /// An observer that keeps the call stack and no events, as a debugger
    /// shows it.
    pub fn call_stack_only() -> Observer {
        Observer {
            events: None,
            ..Observer::new(Filters::default())
        }
    }

    /// Watch as `filters` say from now on, keeping the calls open.
    pub fn set_filters(&mut self, filters: Filters) {
        self.entry_at = entry_addresses(&filters);
        self.filters = filters;
    }

    pub fn filters(&self) -> &Filters {
        &self.filters
    }

    /// The calls open, outermost first.
    pub fn call_stack(&self) -> &[OpenCall] {
        &self.stack
    }

    /// Carry on with these calls open, as after restoring a machine that had
    /// them.
    pub fn restore_call_stack(&mut self, stack: Vec<OpenCall>) {
        self.stack = stack;
        self.dispatch_from = None;
        self.stated = None;
    }

    /// The events since the last take, in order.
    pub fn take_events(&mut self) -> Vec<Event> {
        self.events.as_mut().map(std::mem::take).unwrap_or_default()
    }

    fn event(&mut self, event: Event) {
        if let Some(events) = &mut self.events {
            events.push(event);
        }
    }

    /// Clear the stack, the events and the step under way, as a restored or
    /// reset machine starts afresh.
    pub(crate) fn forget(&mut self) {
        self.restore_call_stack(Vec::new());
        if let Some(events) = &mut self.events {
            events.clear();
        }
        self.step = None;
        self.state = None;
        self.accesses.clear();
    }

    #[inline]
    pub(crate) fn access(&mut self, kind: AccessKind, address: u16, value: u8, edge: u64) {
        let watched = match kind {
            AccessKind::Read => &self.filters.read,
            AccessKind::Write => &self.filters.write,
        };
        if self.step.is_some() && watched.contains(address) {
            self.accesses.push((kind, address, value, edge));
        }
    }

    fn push(&mut self, call: OpenCall, edge: u64) {
        if self.stack.len() == CALL_STACK_DEPTH {
            self.stack.remove(0);
            self.event(Event::Forgot);
        }
        self.stack.push(call);
        self.event(Event::Opened { call, edge });
    }

    fn close(&mut self, edge: u64) {
        if self.stack.pop().is_some() {
            self.event(Event::Closed { edge });
        }
    }

    /// Report the step's accesses, but an instruction's fetch of its own
    /// bytes (`fetched`).
    fn flush(&mut self, doing: Doing, fetched: impl Fn(u16) -> bool) {
        let Some((pc, began)) = self.step.take() else {
            return;
        };
        let Some(events) = &mut self.events else {
            self.accesses.clear();
            return;
        };
        for (kind, address, value, edge) in self.accesses.drain(..) {
            if kind == AccessKind::Read && fetched(address) {
                continue;
            }
            events.push(Event::Access(Access {
                kind,
                address,
                value,
                edge,
                pc,
                began,
                doing,
            }));
        }
    }
}

impl<M: Model> Console<M> {
    /// Watch the console as `observer` says, or stop watching.
    pub fn set_observer(&mut self, observer: Option<Observer>) {
        self.chassis.observer = observer.map(Box::new);
    }

    pub fn observer(&self) -> Option<&Observer> {
        self.chassis.observer.as_deref()
    }

    pub fn observer_mut(&mut self) -> Option<&mut Observer> {
        self.chassis.observer.as_deref_mut()
    }

    /// The shadow call stack, outermost first: empty unless observed.
    pub fn call_stack(&self) -> &[OpenCall] {
        self.observer().map_or(&[], Observer::call_stack)
    }

    /// The ROM bank mapped at `pc`: 0 outside the switchable bank.
    pub fn rom_bank_at(&self, pc: u16) -> u16 {
        if (0x4000..0x8000).contains(&pc) {
            self.cartridge().switchable_rom_bank().unwrap_or(1)
        } else {
            0
        }
    }

    pub(crate) fn observe_before(&mut self) -> Option<Before> {
        let observer = self.chassis.observer.as_deref()?;
        let cpu = &self.chassis.cpu;
        let b = Before {
            pc: cpu.ir_address,
            sp: cpu.stack_pointer,
            halted: cpu.is_halted(),
            dispatching: cpu.in_dispatch(),
            edge: self.chassis.clock.master_edge(),
        };
        let state = (!b.halted && !b.dispatching && observer.filters.registers.contains(b.pc))
            .then(|| CpuState {
                pc: b.pc,
                registers: Registers {
                    a: cpu.a,
                    f: cpu.flags.bits(),
                    b: cpu.b,
                    c: cpu.c,
                    d: cpu.d,
                    e: cpu.e,
                    h: cpu.h,
                    l: cpu.l,
                },
                sp: cpu.stack_pointer,
                ime: cpu.interrupts_enabled(),
                code: [0, 1, 2].map(|i| self.peek(b.pc.wrapping_add(i))),
            });
        let observer = self.chassis.observer.as_deref_mut()?;
        observer.step = Some((b.pc, b.edge));
        observer.accesses.clear();
        observer.state = state;
        Some(b)
    }

    pub(crate) fn observe_after(&mut self, b: Before) {
        let Some(o) = self.chassis.observer.as_deref() else {
            return;
        };
        let cpu = &self.chassis.cpu;
        let (pc, sp) = (cpu.ir_address, cpu.stack_pointer);
        let (in_dispatch, halted) = (cpu.in_dispatch(), cpu.is_halted());
        let now = self.chassis.clock.master_edge();
        let called = sp == b.sp.wrapping_sub(2);
        let normal = !b.dispatching && !in_dispatch && !b.halted;
        // The opcode and the banks are read only where they decide something.
        let reads = o.accesses.iter().any(|a| a.0 == AccessKind::Read);
        let opcode = (reads || (normal && (halted || called))).then(|| self.peek(b.pc));
        let bank_from = if normal && (halted || o.filters.exec.contains(b.pc)) {
            self.rom_bank_at(b.pc)
        } else {
            0
        };
        let bank_to = if b.dispatching || (normal && (called || o.entry_at.contains(pc))) {
            self.rom_bank_at(pc)
        } else {
            0
        };
        let fetched = |a: u16| {
            opcode.is_some_and(|op| (b.pc..b.pc.wrapping_add(instruction_length(op))).contains(&a))
        };
        let o = self.chassis.observer.as_deref_mut().unwrap();
        if b.dispatching {
            o.flush(Doing::Dispatch, fetched);
            let edge = o.dispatch_from.take().unwrap_or(b.edge);
            let call = OpenCall {
                opened: Opened::Interrupt { site: b.pc },
                pc,
                bank: bank_to,
                slot: sp,
            };
            o.push(call, edge);
            return;
        }
        if in_dispatch {
            o.flush(Doing::Dispatch, fetched);
            o.dispatch_from = Some(b.edge);
            let after = o.stated.take();
            o.event(Event::Dispatching {
                edge: b.edge,
                after,
            });
            return;
        }
        if b.halted {
            o.flush(Doing::Halt, fetched);
            return;
        }
        o.stated = None;
        if let Some(state) = o.state.take() {
            o.event(Event::State {
                state,
                edge: b.edge,
            });
            o.stated = Some(b.edge);
        }
        if o.filters.exec.contains(b.pc) {
            o.event(Event::Executed {
                pc: b.pc,
                bank: bank_from,
                edge: b.edge,
            });
        }
        o.flush(Doing::Instruction, fetched);
        if opcode == Some(HALT) && halted {
            o.event(Event::Halted {
                pc: b.pc,
                bank: bank_from,
                edge: b.edge,
            });
        }
        while o.stack.last().is_some_and(|c| c.slot < sp) {
            o.close(now);
        }
        if called && opcode.is_some_and(calls_subroutine) {
            let call = OpenCall {
                opened: Opened::Call { site: b.pc },
                pc,
                bank: bank_to,
                slot: sp,
            };
            o.push(call, b.edge);
        }
        let entry = (bank_to, pc);
        if o.entry_at.contains(pc)
            && o.filters.entries.contains(&entry)
            && o.stack
                .last()
                .is_none_or(|c| c.slot != sp || (c.bank, c.pc) != entry)
        {
            while o
                .stack
                .last()
                .is_some_and(|c| c.opened == Opened::Entry && c.slot == sp)
            {
                o.close(now);
            }
            let call = OpenCall {
                opened: Opened::Entry,
                pc,
                bank: bank_to,
                slot: sp,
            };
            o.push(call, now);
        }
    }
}
