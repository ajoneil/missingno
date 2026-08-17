//! The CPU's state at an instruction boundary, as one capturable value.
//!
//! Between instructions the sequencer is absent, so what survives is the
//! register file, the interrupt latches, and the few pin-and-latch carries the
//! next instruction consults: the /INT level as it was sampled, a pending NMI,
//! the Q latch's source flag, and the address the pins still hold. Mid
//! instruction there is more, and [`Cpu::boundary_state`] refuses rather than
//! naming part of it.

use crate::{Cpu, InterruptMode};

/// Names each carried field once — as the saved value and as the CPU field it
/// comes from — and generates the state, the capture, and the restore together,
/// so the two directions cannot drift apart.
macro_rules! carried_across_boundary {
    ($($(#[$note:meta])* $saved:ident: $ty:ty = $register:ident),* $(,)?) => {
        /// Everything the CPU carries across an instruction boundary. The
        /// recorded bus trace is a diagnostic of the instruction just run, not
        /// state, and is absent here.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub struct CpuState {
            $($(#[$note])* pub $saved: $ty,)*
        }

        impl Cpu {
            /// The state at an instruction boundary; `None` mid-instruction,
            /// where the sequencer holds residue this does not name.
            pub fn boundary_state(&self) -> Option<CpuState> {
                if !self.at_instruction_boundary() {
                    return None;
                }
                Some(CpuState { $($saved: self.$register,)* })
            }

            /// Reseat the CPU at an instruction boundary, discarding any
            /// sequencer in flight and the bus trace that went with it.
            pub fn restore_boundary(&mut self, state: &CpuState) {
                $(self.$register = state.$saved;)*
                self.trace.clear();
                self.sequencer = None;
            }
        }
    };
}

carried_across_boundary! {
    a: u8 = a,
    f: u8 = f,
    b: u8 = b,
    c: u8 = c,
    d: u8 = d,
    e: u8 = e,
    h: u8 = h,
    l: u8 = l,
    /// The alternate set, reached through EX AF,AF' and EXX.
    a_alt: u8 = a_,
    f_alt: u8 = f_,
    b_alt: u8 = b_,
    c_alt: u8 = c_,
    d_alt: u8 = d_,
    e_alt: u8 = e_,
    h_alt: u8 = h_,
    l_alt: u8 = l_,
    ix: u16 = ix,
    iy: u16 = iy,
    sp: u16 = sp,
    pc: u16 = pc,
    /// MEMPTR, which the undocumented flags of BIT n,(HL) read.
    wz: u16 = wz,
    i: u8 = i,
    r: u8 = r,
    iff1: bool = iff1,
    iff2: bool = iff2,
    interrupt_mode: InterruptMode = im,
    halted: bool = halted,
    /// Interrupt acceptance is held off for the instruction following EI.
    ei_pending: bool = ei_pending,
    /// F left by the last flag-modifying instruction, else 0 — SCF/CCF fold it
    /// into their undocumented X/Y flags.
    q: u8 = q,
    /// Whether the instruction just retired touched the flags, which decides
    /// the next Q.
    flags_touched: bool = flags_touched,
    /// Set by LD A,I / LD A,R, whose PF took IFF2.
    p: bool = p,
    nmi_pending: bool = nmi_pending,
    /// /INT as the board drives it, and as acceptance sampled it at the last
    /// instruction's final T-state.
    irq_line: bool = irq_line,
    irq_sampled: bool = irq_sampled,
    /// The address the pins hold; an internal T-state drives it unchanged.
    address_bus: u16 = last_address,
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
