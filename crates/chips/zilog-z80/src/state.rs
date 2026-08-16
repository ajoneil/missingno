//! The CPU's state at an instruction boundary, as one capturable value.
//!
//! Between instructions the sequencer is absent, so what survives is the
//! register file, the interrupt latches, and the few pin-and-latch carries the
//! next instruction consults: the /INT level as it was sampled, a pending NMI,
//! the Q latch's source flag, and the address the pins still hold. Mid
//! instruction there is more, and [`Cpu::boundary_state`] refuses rather than
//! naming part of it.

use crate::{Cpu, InterruptMode};

/// Everything the CPU carries across an instruction boundary. The recorded bus
/// trace is a diagnostic of the instruction just run, not state, and is absent
/// here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CpuState {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    /// The alternate set, reached through EX AF,AF' and EXX.
    pub a_alt: u8,
    pub f_alt: u8,
    pub b_alt: u8,
    pub c_alt: u8,
    pub d_alt: u8,
    pub e_alt: u8,
    pub h_alt: u8,
    pub l_alt: u8,
    pub ix: u16,
    pub iy: u16,
    pub sp: u16,
    pub pc: u16,
    /// MEMPTR, which the undocumented flags of BIT n,(HL) read.
    pub wz: u16,
    pub i: u8,
    pub r: u8,
    pub iff1: bool,
    pub iff2: bool,
    pub interrupt_mode: InterruptMode,
    pub halted: bool,
    /// Interrupt acceptance is held off for the instruction following EI.
    pub ei_pending: bool,
    /// F left by the last flag-modifying instruction, else 0 — SCF/CCF fold it
    /// into their undocumented X/Y flags.
    pub q: u8,
    /// Whether the instruction just retired touched the flags, which decides
    /// the next Q.
    pub flags_touched: bool,
    /// Set by LD A,I / LD A,R, whose PF took IFF2.
    pub p: bool,
    pub nmi_pending: bool,
    /// /INT as the board drives it, and as acceptance sampled it at the last
    /// instruction's final T-state.
    pub irq_line: bool,
    pub irq_sampled: bool,
    /// The address the pins hold; an internal T-state drives it unchanged.
    pub address_bus: u16,
}

impl Cpu {
    /// The state at an instruction boundary; `None` mid-instruction, where the
    /// sequencer holds residue this does not name.
    pub fn boundary_state(&self) -> Option<CpuState> {
        if !self.at_instruction_boundary() {
            return None;
        }
        Some(CpuState {
            a: self.a,
            f: self.f,
            b: self.b,
            c: self.c,
            d: self.d,
            e: self.e,
            h: self.h,
            l: self.l,
            a_alt: self.a_,
            f_alt: self.f_,
            b_alt: self.b_,
            c_alt: self.c_,
            d_alt: self.d_,
            e_alt: self.e_,
            h_alt: self.h_,
            l_alt: self.l_,
            ix: self.ix,
            iy: self.iy,
            sp: self.sp,
            pc: self.pc,
            wz: self.wz,
            i: self.i,
            r: self.r,
            iff1: self.iff1,
            iff2: self.iff2,
            interrupt_mode: self.im,
            halted: self.halted,
            ei_pending: self.ei_pending,
            q: self.q,
            flags_touched: self.flags_touched,
            p: self.p,
            nmi_pending: self.nmi_pending,
            irq_line: self.irq_line,
            irq_sampled: self.irq_sampled,
            address_bus: self.last_address,
        })
    }

    /// Reseat the CPU at an instruction boundary, discarding any sequencer in
    /// flight and the bus trace that went with it.
    pub fn restore_boundary(&mut self, state: &CpuState) {
        self.a = state.a;
        self.f = state.f;
        self.b = state.b;
        self.c = state.c;
        self.d = state.d;
        self.e = state.e;
        self.h = state.h;
        self.l = state.l;
        self.a_ = state.a_alt;
        self.f_ = state.f_alt;
        self.b_ = state.b_alt;
        self.c_ = state.c_alt;
        self.d_ = state.d_alt;
        self.e_ = state.e_alt;
        self.h_ = state.h_alt;
        self.l_ = state.l_alt;
        self.ix = state.ix;
        self.iy = state.iy;
        self.sp = state.sp;
        self.pc = state.pc;
        self.wz = state.wz;
        self.i = state.i;
        self.r = state.r;
        self.iff1 = state.iff1;
        self.iff2 = state.iff2;
        self.im = state.interrupt_mode;
        self.halted = state.halted;
        self.ei_pending = state.ei_pending;
        self.q = state.q;
        self.flags_touched = state.flags_touched;
        self.p = state.p;
        self.nmi_pending = state.nmi_pending;
        self.irq_line = state.irq_line;
        self.irq_sampled = state.irq_sampled;
        self.last_address = state.address_bus;
        self.trace.clear();
        self.sequencer = None;
    }
}

#[cfg(test)]
mod tests {
    use crate::{Bus, Cpu};

    struct Ram([u8; 0x10000]);

    impl Bus for Ram {
        fn read(&mut self, address: u16) -> u8 {
            self.0[address as usize]
        }
        fn write(&mut self, address: u16, data: u8) {
            self.0[address as usize] = data;
        }
        fn input(&mut self, _port: u16) -> u8 {
            0xFF
        }
        fn output(&mut self, _port: u16, _data: u8) {}
    }

    fn program(bytes: &[u8]) -> Ram {
        let mut ram = Ram([0; 0x10000]);
        ram.0[..bytes.len()].copy_from_slice(bytes);
        ram
    }

    #[test]
    fn a_restored_cpu_runs_the_same_instructions() {
        // EX AF,AF' / EXX / LD A,I / EI, so the alternate set, WZ, Q and the
        // interrupt latches all differ from power-on at the save point.
        let code = [
            0x3E, 0x42, 0x08, 0x01, 0x34, 0x12, 0xD9, 0xED, 0x47, 0xED, 0x57, 0xFB, 0x21, 0x00,
            0xC0, 0x36, 0x99, 0x23, 0x7E,
        ];
        let mut bus = program(&code);
        let mut cpu = Cpu::new();
        for _ in 0..6 {
            cpu.step(&mut bus);
        }

        let state = cpu.boundary_state().expect("stepped to a boundary");
        let mut restored = Cpu::new();
        restored.restore_boundary(&state);
        let mut restored_bus = program(&code);

        for _ in 0..5 {
            cpu.step(&mut bus);
            restored.step(&mut restored_bus);
            assert_eq!(restored.boundary_state(), cpu.boundary_state());
        }
        assert_eq!(restored_bus.0[0xC000], bus.0[0xC000]);
    }

    #[test]
    fn a_mid_instruction_cpu_has_no_boundary_state() {
        let mut bus = program(&[0x21, 0x00, 0xC0]);
        let mut cpu = Cpu::new();
        cpu.tick(&mut bus);
        assert!(cpu.boundary_state().is_none());
    }
}
