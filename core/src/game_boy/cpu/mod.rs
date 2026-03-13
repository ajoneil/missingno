use flags::{Flag, Flags};
use registers::{Register8, Register16};

pub mod flags;
pub mod instructions;
pub mod mcycle;
pub mod registers;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum InterruptMasterEnable {
    Disabled,
    Enabled,
}

/// Models the CPU's execution state with respect to the HALT instruction.
///
/// On hardware, HALT puts the CPU into a low-power idle loop where it
/// continues to tick hardware (PPU, timers, etc.) each M-cycle but
/// doesn't execute instructions. When `(IF & IE) != 0`, the DFF cascade
/// (g42 → g43 → g49) propagates within the idle M-cycle, but PHI doesn't
/// resume until the next M-cycle — the wakeup NOP. The `InterruptLatch`
/// enum's Fresh→Ready promotion at step() entry naturally models this
/// 1 M-cycle propagation delay.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum HaltState {
    /// Normal execution — CPU fetches and executes instructions.
    Running,
    /// HALT instruction decoded — the trailing fetch runs as a dummy
    /// fetch (read [PC] without incrementing), then transitions to
    /// Halted. Models the hardware's 1 M-cycle transition from HALT
    /// execution to idle mode.
    Halting,
    /// HALT idle loop — ticking hardware, waiting for `(IF & IE) != 0`.
    Halted,
}

/// Models the EI instruction's one-instruction delay DFF pipeline.
///
/// On hardware, EI sets an intermediate RS latch in the sequencer.
/// That latch propagates through a DFF chain (clocked by CLK9) before
/// reaching the IME flip-flop, introducing a one-instruction delay.
/// This enum represents the pipeline stages visible at instruction
/// boundaries.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EiDelay {
    /// EI was executed this instruction. Will advance to Fired at this
    /// instruction's completion boundary. IME is not yet promoted.
    Pending,

    /// EI's delay has advanced one stage. IME will be promoted to
    /// Enabled at this instruction's completion boundary (i.e., the
    /// instruction after EI).
    Fired,
}

#[derive(Clone)]
pub struct Cpu {
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,

    pub stack_pointer: u16,
    pub program_counter: u16,

    pub flags: Flags,

    pub interrupt_master_enable: InterruptMasterEnable,
    /// EI delay pipeline — models the DFF cascade between EI's
    /// decode signal and the IME flip-flop. Advances one stage per
    /// instruction completion in `advance_ei_delay()`.
    pub ei_delay: Option<EiDelay>,
    pub halt_state: HaltState,
    /// HALT bug: when HALT is executed with IME=0 and an interrupt is
    /// already pending, the CPU doesn't truly halt — it resumes
    /// immediately but fails to increment PC on the next opcode fetch,
    /// causing that byte to be read twice.
    pub halt_bug: bool,
}

impl Cpu {
    pub fn new(checksum: u8) -> Cpu {
        Cpu {
            a: 0x01,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xd8,
            h: 0x01,
            l: 0x4d,

            stack_pointer: 0xfffe,
            program_counter: 0x0100,

            flags: if checksum == 0 {
                Flags::ZERO
            } else {
                Flags::ZERO | Flags::CARRY | Flags::HALF_CARRY
            },

            interrupt_master_enable: InterruptMasterEnable::Disabled,
            ei_delay: None,
            halt_state: HaltState::Running,
            halt_bug: false,
        }
    }

    pub fn get_register8(&self, register: Register8) -> u8 {
        match register {
            Register8::A => self.a,
            Register8::B => self.b,
            Register8::C => self.c,
            Register8::D => self.d,
            Register8::E => self.e,
            Register8::H => self.h,
            Register8::L => self.l,
        }
    }

    pub(crate) fn set_register8(&mut self, register: Register8, value: u8) {
        match register {
            Register8::A => self.a = value,
            Register8::B => self.b = value,
            Register8::C => self.c = value,
            Register8::D => self.d = value,
            Register8::E => self.e = value,
            Register8::H => self.h = value,
            Register8::L => self.l = value,
        }
    }

    pub fn get_register16(&self, register: Register16) -> u16 {
        match register {
            Register16::Bc => u16::from_be_bytes([self.b, self.c]),
            Register16::De => u16::from_be_bytes([self.d, self.e]),
            Register16::Hl => u16::from_be_bytes([self.h, self.l]),
            Register16::StackPointer => self.stack_pointer,
            Register16::Af => u16::from_be_bytes([self.a, self.flags.bits()]),
        }
    }

    pub(crate) fn set_register16(&mut self, register: Register16, value: u16) {
        let high = (value / 0x100) as u8;
        let low = (value % 0x100) as u8;

        match register {
            Register16::Bc => {
                self.b = high;
                self.c = low;
            }
            Register16::De => {
                self.d = high;
                self.e = low;
            }
            Register16::Hl => {
                self.h = high;
                self.l = low;
            }
            Register16::StackPointer => self.stack_pointer = value,
            Register16::Af => {
                self.a = high;
                self.flags = Flags::from_bits_retain(low & 0xF0);
            }
        }
    }

    pub fn interrupts_enabled(&self) -> bool {
        self.interrupt_master_enable != InterruptMasterEnable::Disabled
    }
}
