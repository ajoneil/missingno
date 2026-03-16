use flags::{Flag, Flags};
use mcycle::{BusDot, CpuPhase};
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
/// resume until the next M-cycle — the wakeup NOP. The
/// `first_halted_cycle` flag models this 1 M-cycle propagation delay.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum HaltState {
    /// Normal execution — CPU fetches and executes instructions.
    Running,
    /// HALT instruction decoded — the next fetch runs as a dummy
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

/// Models the CPU's interrupt dispatch latch (g42 DFF).
///
/// On hardware, the g42 DFF samples `IF & IE` combinationally at the
/// M-cycle boundary. The PPU fires IF and the CPU checks dispatch at
/// the same edge — there is no multi-cycle pipeline delay. The
/// executor ensures `update_interrupt_state` runs after PPU rise and
/// before the CPU's M-cycle transition so that `take_ready()` sees
/// interrupts from the current boundary.
#[derive(Clone, Copy)]
pub(super) enum InterruptLatch {
    /// No interrupt pending.
    Empty,
    /// Interrupt ready for dispatch. `take_ready()` consumes it.
    Ready(super::interrupts::Interrupt),
}

impl InterruptLatch {
    /// Take the interrupt if ready. Returns None for Empty.
    pub(super) fn take_ready(&mut self) -> Option<super::interrupts::Interrupt> {
        if let InterruptLatch::Ready(interrupt) = *self {
            *self = InterruptLatch::Empty;
            Some(interrupt)
        } else {
            None
        }
    }
}

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
    /// IME=0 HALT wakeup pending flag. Set by `update_interrupt_state`
    /// when an interrupt fires during HALT with IME=0. Consumed by
    /// `mcycle_halted` on the next M-cycle boundary, modeling the
    /// hardware's CLK_ENA re-enable delay (g42 latches during the idle
    /// M-cycle, but the CPU doesn't resume clocking until the next
    /// M-cycle boundary).
    pub(super) halt_wakeup_pending: bool,

    // ── Persistent state machine fields ──
    /// Current execution phase of the CPU state machine.
    pub(super) phase: CpuPhase,
    /// The decoded instruction, preserved for debugger display.
    pub(super) instruction: instructions::Instruction,
    /// Dot position within the current M-cycle (0–3).
    pub(super) dot: BusDot,
    /// Whether we have a pending M-cycle.
    pub(super) mcycle_active: bool,
    /// The BusAction for the current M-cycle.
    current_action: Option<mcycle::BusAction>,
    /// Step counter for Fetch/Halted phases (tracks M-cycle sub-steps).
    pub(super) exec_step: u8,
    /// Scratch byte for multi-read phases (Pop, CondReturn).
    pub(super) scratch: u8,
    /// The dot position that produced the last DotAction (for the executor
    /// to check timing signals like boga, bowa, mopa).
    pub(super) last_dot: BusDot,
    /// IE push bug flag.
    pub(super) pending_vector_resolve: bool,
    /// Interrupt latch (g42 DFF).
    pub(super) interrupt_latch: InterruptLatch,
    /// Set when the CPU transitions to the Fetch phase. The executor
    /// reads this to detect instruction boundaries for EI delay and
    /// step_instruction().
    pub(super) boundary_flag: bool,
    /// Set when the CPU transitions from the dummy fetch into HALT
    /// idle mode. When true, the first `mcycle_halted()` call skips
    /// the interrupt check and emits the wakeup NOP unconditionally.
    /// Models the hardware constraint that g42's output from the HALT
    /// entry M-cycle hasn't propagated through g43/g49 yet.
    pub(super) first_halted_cycle: bool,
    /// Whether an interrupt is currently pending (IF & IE != 0).
    /// Updated every dot by `update_interrupt_state`. Used by the CPU
    /// state machine for the HALT bug check.
    pub(super) interrupt_pending: bool,
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
            halt_wakeup_pending: false,

            phase: CpuPhase::Fetch,
            instruction: instructions::Instruction::NoOperation,
            dot: BusDot::ZERO,
            mcycle_active: false,
            current_action: None,
            exec_step: 0,
            scratch: 0,
            last_dot: BusDot::ZERO,
            pending_vector_resolve: false,
            interrupt_latch: InterruptLatch::Empty,
            boundary_flag: true, // Start at an instruction boundary
            first_halted_cycle: false,
            interrupt_pending: false,
        }
    }

    /// Power-on state: all registers zeroed, PC=0x0000 for boot ROM entry.
    pub fn power_on() -> Cpu {
        Cpu {
            a: 0,
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            h: 0,
            l: 0,
            stack_pointer: 0x0000,
            program_counter: 0x0000,
            flags: Flags::empty(),
            interrupt_master_enable: InterruptMasterEnable::Disabled,
            ei_delay: None,
            halt_state: HaltState::Running,
            halt_bug: false,
            halt_wakeup_pending: false,
            phase: CpuPhase::Fetch,
            instruction: instructions::Instruction::NoOperation,
            dot: BusDot::ZERO,
            mcycle_active: false,
            current_action: None,
            exec_step: 0,
            scratch: 0,
            last_dot: BusDot::ZERO,
            pending_vector_resolve: false,
            interrupt_latch: InterruptLatch::Empty,
            boundary_flag: true,
            first_halted_cycle: false,
            interrupt_pending: false,
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
