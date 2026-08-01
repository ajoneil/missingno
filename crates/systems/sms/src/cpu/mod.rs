//! Zilog NMOS Z80 core, driven a whole instruction per `step`.
//!
//! Granularity: the core executes one instruction at a time, but records
//! its bus activity at T-state resolution. Each machine cycle (opcode
//! fetch, memory read/write, I/O read/write, internal delay) appends the
//! per-T-state address/data/control-pin snapshots the SingleStepTests
//! oracle samples between cycles, using the oracle's simplified memory
//! timing (MREQ/RD/WR pulse for a single T-state). `step` returns the
//! T-state count so the console can advance the VDP by the matching number
//! of dots; `bus_trace` exposes the recorded snapshots for verification.
//!
//! Interrupt entry (NMI, IM 0/1/2 maskable) implements the documented
//! acceptance semantics, but the SingleStepTests set contains no interrupt
//! cases, so its cycle-level timing is oracle-unverified.

mod apply;
pub mod decode;
mod execute;

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
        }
    }

    pub fn trigger_nmi(&mut self) {
        self.nmi_pending = true;
    }

    pub fn set_irq(&mut self, asserted: bool) {
        self.irq_line = asserted;
    }

    /// The bus snapshots recorded during the most recent `step`.
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

    /// Execute one instruction (or accept a pending interrupt). Returns the
    /// number of T-states consumed.
    pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
        self.trace.clear();

        if self.nmi_pending {
            self.nmi_pending = false;
            self.accept_nmi(bus);
            return self.trace.len() as u32;
        }
        if self.irq_line && self.iff1 && !self.ei_pending {
            self.accept_irq(bus);
            return self.trace.len() as u32;
        }

        self.ei_pending = false;

        if self.halted {
            // HALT re-fetches its successor byte each cycle without
            // advancing PC; execution resumes when an interrupt lands.
            let pc = self.pc;
            self.opcode_fetch(bus);
            self.pc = pc;
            return self.trace.len() as u32;
        }

        self.flags_touched = false;
        self.p = false;
        let opcode = self.opcode_fetch(bus);
        self.execute(bus, opcode);
        self.q = if self.flags_touched { self.f } else { 0 };
        self.trace.len() as u32
    }

    fn tick(&mut self, address: u16, data: Option<u8>, pins: Pins) {
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
        let pc = self.pc;
        self.tick(pc, None, Pins::IDLE);
        self.tick(pc, None, Pins::MEM_READ);
        let opcode = bus.read(pc);
        self.pc = pc.wrapping_add(1);
        let refresh = self.refresh_address();
        self.tick(refresh, Some(opcode), Pins::IDLE);
        self.tick(refresh, None, Pins::IDLE);
        self.inc_r();
        opcode
    }

    fn mem_read(&mut self, bus: &mut impl Bus, address: u16) -> u8 {
        self.tick(address, None, Pins::IDLE);
        self.tick(address, None, Pins::MEM_READ);
        let data = bus.read(address);
        self.tick(address, Some(data), Pins::IDLE);
        data
    }

    fn mem_write(&mut self, bus: &mut impl Bus, address: u16, data: u8) {
        self.tick(address, None, Pins::IDLE);
        self.tick(address, Some(data), Pins::MEM_WRITE);
        bus.write(address, data);
        self.tick(address, None, Pins::IDLE);
    }

    fn io_read(&mut self, bus: &mut impl Bus, port: u16) -> u8 {
        self.tick(port, None, Pins::IDLE);
        self.tick(port, None, Pins::IDLE);
        self.tick(port, None, Pins::IO_READ);
        let data = bus.input(port);
        self.tick(port, Some(data), Pins::IDLE);
        data
    }

    fn io_write(&mut self, bus: &mut impl Bus, port: u16, data: u8) {
        self.tick(port, None, Pins::IDLE);
        self.tick(port, None, Pins::IDLE);
        self.tick(port, Some(data), Pins::IO_WRITE);
        bus.output(port, data);
        self.tick(port, None, Pins::IDLE);
    }

    /// `count` internal T-states, holding the last driven address.
    fn internal(&mut self, count: u32) {
        for _ in 0..count {
            self.tick(self.last_address, None, Pins::IDLE);
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
        self.tick(pc, None, Pins::IDLE);
        self.tick(pc, None, Pins::IDLE);
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
