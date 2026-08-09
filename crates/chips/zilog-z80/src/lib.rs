//! Zilog NMOS Z80 core, driven one T-state per `tick`.
//!
//! The core records its bus activity at T-state resolution. Each machine
//! cycle (opcode fetch, memory read/write, I/O read/write, internal delay)
//! appends the per-T-state address/data/control-pin snapshots the
//! SingleStepTests oracle samples between cycles, using the oracle's
//! simplified memory timing (MREQ/RD/WR pulse for a single T-state).
//! `bus_trace` exposes the snapshots recorded for the current instruction;
//! `step` runs a whole instruction and returns its T-state count so a
//! console can advance the VDP by the matching number of dots.
//!
//! Granularity limit: the unprefixed main table walks its own T-states, each
//! bus access landing on the tick its pins assert. The CB/ED/DD/FD prefixes,
//! HALT's re-fetch loop and interrupt acceptance still execute atomically on
//! the T that latches their opcode, handing the remaining T-states back as
//! already-recorded padding — a board interleaving other chips between ticks
//! sees those instructions' accesses bunched at that tick.
//!
//! Interrupt entry (NMI, IM 0/1/2 maskable) implements the documented
//! acceptance semantics, but the SingleStepTests set contains no interrupt
//! cases, so its cycle-level timing is oracle-unverified.

mod apply;
pub mod decode;
pub mod disasm;
mod execute;
pub mod isa;
mod sequencer;

use sequencer::{Cycle, Sequencer};

pub use isa::Z80;

pub mod flags {
    pub const CARRY: u8 = 0x01;
    pub const SUBTRACT: u8 = 0x02;
    pub const PARITY: u8 = 0x04;
    pub const X: u8 = 0x08;
    pub const HALF: u8 = 0x10;
    pub const Y: u8 = 0x20;
    pub const ZERO: u8 = 0x40;
    pub const SIGN: u8 = 0x80;
    pub const XY: u8 = X | Y;
}

pub trait Bus {
    fn read(&mut self, address: u16) -> u8;
    fn write(&mut self, address: u16, data: u8);
    fn input(&mut self, port: u16) -> u8;
    fn output(&mut self, port: u16, data: u8);
}

/// The control pins the oracle samples: RD, WR, MREQ, IORQ.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Pins {
    pub read: bool,
    pub write: bool,
    pub mreq: bool,
    pub iorq: bool,
}

impl Pins {
    const IDLE: Pins = Pins {
        read: false,
        write: false,
        mreq: false,
        iorq: false,
    };
    const MEM_READ: Pins = Pins {
        read: true,
        write: false,
        mreq: true,
        iorq: false,
    };
    const MEM_WRITE: Pins = Pins {
        read: false,
        write: true,
        mreq: true,
        iorq: false,
    };
    const IO_READ: Pins = Pins {
        read: true,
        write: false,
        mreq: false,
        iorq: true,
    };
    const IO_WRITE: Pins = Pins {
        read: false,
        write: true,
        mreq: false,
        iorq: true,
    };
}

/// One T-state's bus snapshot. `data` is `None` when the data pins are
/// electrically disconnected (idle T-states and address-only phases).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BusCycle {
    pub address: u16,
    pub data: Option<u8>,
    pub pins: Pins,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InterruptMode {
    Mode0,
    Mode1,
    Mode2,
}

pub struct Cpu {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub a_: u8,
    pub f_: u8,
    pub b_: u8,
    pub c_: u8,
    pub d_: u8,
    pub e_: u8,
    pub h_: u8,
    pub l_: u8,
    pub ix: u16,
    pub iy: u16,
    pub sp: u16,
    pub pc: u16,
    pub wz: u16,
    pub i: u8,
    pub r: u8,
    pub iff1: bool,
    pub iff2: bool,
    pub im: InterruptMode,
    pub halted: bool,
    /// Set for the instruction following EI: interrupt acceptance is held
    /// off for one instruction after IFF1 goes high.
    pub ei_pending: bool,
    /// F left by the last flag-modifying instruction, else 0 — the term
    /// SCF/CCF fold into their undocumented X/Y flags.
    pub q: u8,
    /// True after LD A,I / LD A,R (whose PF reflects IFF2).
    pub p: bool,
    flags_touched: bool,
    nmi_pending: bool,
    irq_line: bool,
    last_address: u16,
    trace: Vec<BusCycle>,
    /// T-states of the in-flight instruction already recorded in `trace` but
    /// not yet handed back by `tick` — zero at an instruction boundary.
    recorded_ahead: u32,
    /// The instruction walking its T-states, absent between instructions.
    sequencer: Option<Sequencer>,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
            a: 0xFF,
            f: 0xFF,
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            h: 0,
            l: 0,
            a_: 0,
            f_: 0,
            b_: 0,
            c_: 0,
            d_: 0,
            e_: 0,
            h_: 0,
            l_: 0,
            ix: 0,
            iy: 0,
            sp: 0xFFFF,
            pc: 0,
            wz: 0,
            i: 0,
            r: 0,
            iff1: false,
            iff2: false,
            im: InterruptMode::Mode0,
            halted: false,
            ei_pending: false,
            q: 0,
            p: false,
            flags_touched: false,
            nmi_pending: false,
            irq_line: false,
            last_address: 0,
            trace: Vec::new(),
            recorded_ahead: 0,
            sequencer: None,
        }
    }

    pub fn trigger_nmi(&mut self) {
        self.nmi_pending = true;
    }

    pub fn set_irq(&mut self, asserted: bool) {
        self.irq_line = asserted;
    }

    /// The bus snapshots recorded for the most recent instruction.
    pub fn bus_trace(&self) -> &[BusCycle] {
        &self.trace
    }

    pub fn bc(&self) -> u16 {
        u16::from_be_bytes([self.b, self.c])
    }
    pub fn de(&self) -> u16 {
        u16::from_be_bytes([self.d, self.e])
    }
    pub fn hl(&self) -> u16 {
        u16::from_be_bytes([self.h, self.l])
    }
    pub fn af(&self) -> u16 {
        u16::from_be_bytes([self.a, self.f])
    }
    fn set_bc(&mut self, value: u16) {
        [self.b, self.c] = value.to_be_bytes();
    }
    fn set_de(&mut self, value: u16) {
        [self.d, self.e] = value.to_be_bytes();
    }
    fn set_hl(&mut self, value: u16) {
        [self.h, self.l] = value.to_be_bytes();
    }

    /// Between instructions — the debugger's stepping boundary, and where
    /// interrupts are sampled.
    pub fn at_instruction_boundary(&self) -> bool {
        self.sequencer.is_none() && self.recorded_ahead == 0
    }

    /// Advance one T-state. A sequenced instruction records exactly this
    /// T-state's bus snapshot and fires any bus call it carries; an
    /// unsequenced one (a prefix, HALT's re-fetch, an accepted interrupt)
    /// records its remainder at once and hands those T-states back one per
    /// call, touching neither bus nor trace.
    pub fn tick(&mut self, bus: &mut impl Bus) {
        if self.recorded_ahead > 0 {
            self.recorded_ahead -= 1;
            return;
        }
        if self.sequencer.is_none() {
            self.trace.clear();
        }
        let recorded = self.trace.len();
        self.advance(bus);
        self.recorded_ahead = (self.trace.len() - recorded) as u32 - 1;
    }

    fn advance(&mut self, bus: &mut impl Bus) {
        let mut sequencer = match self.sequencer.take() {
            Some(sequencer) => sequencer,
            None => match self.begin_instruction(bus) {
                Some(sequencer) => sequencer,
                None => return,
            },
        };
        if self.tick_sequencer(bus, &mut sequencer) {
            self.sequencer = Some(sequencer);
        } else {
            self.q = if self.flags_touched { self.f } else { 0 };
        }
    }

    /// Sample interrupts and start M1, or run one of the entry sequences that
    /// have no sequencer plan yet.
    fn begin_instruction(&mut self, bus: &mut impl Bus) -> Option<Sequencer> {
        if self.nmi_pending {
            self.nmi_pending = false;
            self.accept_nmi(bus);
            return None;
        }
        if self.irq_line && self.iff1 && !self.ei_pending {
            self.accept_irq(bus);
            return None;
        }

        self.ei_pending = false;

        if self.halted {
            // HALT re-fetches its successor byte each cycle without
            // advancing PC; execution resumes when an interrupt lands.
            let pc = self.pc;
            self.opcode_fetch(bus);
            self.pc = pc;
            return None;
        }

        self.flags_touched = false;
        self.p = false;
        Some(Sequencer::fetching())
    }

    /// Run to the next instruction boundary. Returns the number of T-states
    /// consumed.
    pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
        if self.at_instruction_boundary() {
            self.tick(bus);
        }
        while !self.at_instruction_boundary() {
            self.tick(bus);
        }
        self.trace.len() as u32
    }

    fn record(&mut self, address: u16, data: Option<u8>, pins: Pins) {
        self.last_address = address;
        self.trace.push(BusCycle {
            address,
            data,
            pins,
        });
    }

    fn inc_r(&mut self) {
        self.r = (self.r & 0x80) | (self.r.wrapping_add(1) & 0x7F);
    }

    fn refresh_address(&self) -> u16 {
        u16::from_be_bytes([self.i, self.r])
    }

    /// M1: fetch the opcode at PC, drive the refresh address, bump R.
    fn opcode_fetch(&mut self, bus: &mut impl Bus) -> u8 {
        self.run_cycle(bus, Cycle::OpcodeFetch)
    }

    fn mem_read(&mut self, bus: &mut impl Bus, address: u16) -> u8 {
        self.run_cycle(bus, Cycle::MemRead { address })
    }

    fn mem_write(&mut self, bus: &mut impl Bus, address: u16, data: u8) {
        self.run_cycle(bus, Cycle::MemWrite { address, data });
    }

    fn io_read(&mut self, bus: &mut impl Bus, port: u16) -> u8 {
        self.run_cycle(bus, Cycle::IoRead { port })
    }

    fn io_write(&mut self, bus: &mut impl Bus, port: u16, data: u8) {
        self.run_cycle(bus, Cycle::IoWrite { port, data });
    }

    fn internal_tick(&mut self) {
        self.record(self.last_address, None, Pins::IDLE);
    }

    /// `count` internal T-states, holding the last driven address.
    fn internal(&mut self, count: u32) {
        for _ in 0..count {
            self.internal_tick();
        }
    }

    fn read16(&mut self, bus: &mut impl Bus, address: u16) -> u16 {
        let lo = self.mem_read(bus, address);
        let hi = self.mem_read(bus, address.wrapping_add(1));
        u16::from_le_bytes([lo, hi])
    }

    fn push16(&mut self, bus: &mut impl Bus, value: u16) {
        let [lo, hi] = value.to_le_bytes();
        self.sp = self.sp.wrapping_sub(1);
        self.mem_write(bus, self.sp, hi);
        self.sp = self.sp.wrapping_sub(1);
        self.mem_write(bus, self.sp, lo);
    }

    fn pop16(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = self.mem_read(bus, self.sp);
        self.sp = self.sp.wrapping_add(1);
        let hi = self.mem_read(bus, self.sp);
        self.sp = self.sp.wrapping_add(1);
        u16::from_le_bytes([lo, hi])
    }

    fn accept_nmi(&mut self, bus: &mut impl Bus) {
        self.halted = false;
        self.iff1 = false;
        let pc = self.pc;
        self.record(pc, None, Pins::IDLE);
        self.record(pc, None, Pins::IDLE);
        self.inc_r();
        self.internal(1);
        self.push16(bus, pc);
        self.pc = 0x0066;
        self.wz = 0x0066;
    }

    fn accept_irq(&mut self, bus: &mut impl Bus) {
        self.halted = false;
        self.iff1 = false;
        self.iff2 = false;
        self.inc_r();
        let pc = self.pc;
        match self.im {
            InterruptMode::Mode0 | InterruptMode::Mode1 => {
                self.internal(2);
                self.push16(bus, pc);
                self.pc = 0x0038;
                self.wz = 0x0038;
            }
            InterruptMode::Mode2 => {
                self.internal(2);
                self.push16(bus, pc);
                let vector = u16::from_be_bytes([self.i, 0xFF]);
                let target = self.read16(bus, vector);
                self.pc = target;
                self.wz = target;
            }
        }
    }
}

impl<B: Bus> missingno_core::ClockedCpu<B> for Cpu {
    fn tick(&mut self, bus: &mut B) {
        Cpu::tick(self, bus);
    }

    fn at_instruction_boundary(&self) -> bool {
        Cpu::at_instruction_boundary(self)
    }
}
