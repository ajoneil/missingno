use super::instructions::CarryFlag;
use super::instructions::bit_shift::{Carry, Direction};
use super::mcycle::AluOp;
use super::registers::{Register8, Register16};

/// The retire-edge mutation an instruction produces at the end-of-instruction
/// CLK9↑ — the specific set of DFF captures (register file, IME, halt state)
/// that fire for this opcode on hardware.
///
/// Produced by `decode` for single-M-cycle instructions (Phase::Empty arms)
/// or by the terminal step of a multi-M-cycle Phase; consumed by
/// `Cpu::commit`, which reads pre-edge state (via `ime.output()` for
/// `dispatch_trigger`) before dispatching the variant-specific mutation.
///
/// Each variant corresponds to a specific DFF-capture pattern. Variants for
/// multi-M-cycle terminal-step commits reuse single-M-cycle variants where
/// the shape matches (e.g., `LD r,[HL]` final step emits `Commit::LoadR8`
/// just as `LD r,d8` does).
#[allow(dead_code)]
#[derive(Debug)]
pub(super) enum Commit {
    // ── No register/flag change ──
    /// Retire edge with no architectural mutation. NOP, not-taken
    /// conditional branches, and multi-M-cycle instructions whose work
    /// has already executed at decode-edge inline sites.
    NoOperation,
    /// Invalid opcode — enters `HaltState::Locked` (hard-lock until
    /// power-off). Hardware continues to tick; the CPU never resumes.
    Invalid,

    // ── 8-bit register writes ──
    LoadR8 {
        reg: Register8,
        value: u8,
    },
    IncR8 {
        reg: Register8,
    },
    DecR8 {
        reg: Register8,
    },
    AluA {
        op: AluOp,
        value: u8,
    },

    // ── 16-bit register writes ──
    LoadR16 {
        reg: Register16,
        value: u16,
    },
    Inc16 {
        reg: Register16,
    },
    Dec16 {
        reg: Register16,
    },
    AddHl {
        source: Register16,
    },
    AddSpOffset {
        offset: i8,
    },
    LdHlSpOffset {
        offset: i8,
    },

    // ── Flags / accumulator bit ops ──
    Daa,
    CarryFlag(CarryFlag),
    ComplementA,
    RotateAccumulator {
        direction: Direction,
        carry: Carry,
    },

    // ── CB-prefixed register ops ──
    RotateReg {
        reg: Register8,
        direction: Direction,
        carry: Carry,
    },
    ShiftArithmetical {
        reg: Register8,
        direction: Direction,
    },
    ShiftRightLogical {
        reg: Register8,
    },
    SwapReg {
        reg: Register8,
    },
    BitTest {
        bit: u8,
        reg: Register8,
    },
    BitSet {
        bit: u8,
        reg: Register8,
    },
    BitReset {
        bit: u8,
        reg: Register8,
    },

    // ── Interrupt control ──
    /// DI — clears IME and ime_delay immediately.
    DisableInterrupts,
    /// EI — sets ime_delay only. The next M-cycle boundary copies
    /// ime_delay → ime, which is the "1-instruction delay" before
    /// dispatch can observe the enable.
    EnableInterrupts,

    // ── Halt / stop / low-power entry ──
    EnterHalt,
    EnterStop,
}
