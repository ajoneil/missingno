use super::instructions::bit_shift::{Carry, Direction};
use super::instructions::CarryFlag;
use super::mcycle::AluOp;
use super::registers::{Register16, Register8};

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
    /// Invalid opcode — enters halt_state = Halting per current emulator
    /// behaviour. Not a hardware signal; the SM83 locks up on invalid
    /// opcodes.
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
    /// DI — IME DFF (zivv) ← Disabled. Hardware: zwuu clears the
    /// ime_pending (zjje) SR latch combinationally during DI's data_phase.
    /// The dispatch_active chain (zaij/zkog) is gated this M-cycle.
    DisableInterrupts,
    /// EI — IME DFF (zivv) ← Enabled. Hardware: zbpp sets the ime_pending
    /// (zjje) SR latch combinationally during EI's data_phase. The
    /// dispatch_active chain (zaij/zkog) is gated this M-cycle — source
    /// of the "1-instruction delay" before dispatch.
    EnableInterrupts,

    // ── Halt / stop / low-power entry ──
    EnterHalt,
    EnterStop,
}
