//! Types used by the M-cycle scheduler and the per-phase modules.

use super::super::commit::Commit;
use super::super::instructions::bit_shift::{Carry, Direction};
use super::super::registers::{Register8, Register16};

// ── Bus action ──────────────────────────────────────────────────────────

/// What happens on the memory bus during one M-cycle.
#[derive(Debug)]
pub(crate) enum MCycleAction {
    /// Read a byte at the given address.
    Read { address: u16 },
    /// Write a byte to the given address.
    Write { address: u16, value: u8 },
    /// No bus activity (internal CPU work). The address stays on the
    /// bus pins from the previous request (hardware cpu_bus_pass).
    Internal { address: u16 },
    /// Internal cycle where the IDU places an address on the bus,
    /// potentially triggering the DMG OAM corruption bug if the
    /// address is in 0xFE00-0xFEFF and the PPU is in Mode 2.
    InternalOamBug { address: u16 },
}

// ── T-cycle position within the M-cycle ─────────────────────────────────

/// Position of the CPU's ring counter within an M-cycle: 0–3 (four
/// T-cycles per M-cycle). Driven by the master clock; ticked by the
/// SM83's internal AFUR/ALEF/APUK/ADYK DFFs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TCycle(u8);

impl TCycle {
    pub const ZERO: TCycle = TCycle(0);
    pub const ONE: TCycle = TCycle(1);

    /// Raw T-cycle number (0–3).
    pub fn as_u8(self) -> u8 {
        self.0
    }

    pub fn advance(self) -> TCycle {
        debug_assert!(self.0 < 3, "cannot advance past T-cycle 3");
        TCycle(self.0 + 1)
    }
}

// ── T-cycle-level bus state ─────────────────────────────────────────────

/// What the CPU bus is doing during one T-cycle.
///
/// The executor ticks all hardware and routes these to subsystems.
/// Each M-cycle expands into 4 T-cycles with bus operations placed
/// at the hardware-correct position:
/// - **Read**:     `[Idle, Idle, Idle, Read]`
/// - **Write**:    `[Idle, Idle, Idle, Write]`
/// - **Internal**: `[Idle, Idle, Idle, Idle]`
/// - **OamBug**:   `[InternalOamBug, Idle, Idle, Idle]`
#[derive(Clone, Copy, Debug)]
pub enum BusAction {
    /// No bus transfer this T-cycle.
    Idle,
    /// CPU is reading from this address. The executor must provide
    /// the value before the next M-cycle begins.
    Read { address: u16 },
    /// CPU is writing this value to this address. The write latches
    /// at this T-cycle (G→H boundary, end of M-cycle).
    Write { address: u16, value: u8 },
    /// Internal cycle where the IDU places an address on the bus.
    /// May trigger OAM bug if address is in 0xFE00-0xFEFF.
    InternalOamBug { address: u16 },
}

// ── Helper enums ────────────────────────────────────────────────────────

/// ALU operation applied to A with a read value.
#[derive(Clone, Copy, Debug)]
pub(crate) enum AluOp {
    Add,
    Sub,
    Adc { carry: u8 },
    Sbc { carry: u8 },
    Cp,
    And,
    Or,
    Xor,
}

/// What to do after reading one byte from memory.
#[derive(Debug)]
pub(crate) enum ReadAction {
    /// Load into register.
    LoadRegister(Register8),
    /// Load into register, then adjust HL.
    LoadRegisterHlPost(Register8, i16),
    /// Apply ALU op with A.
    AluA(AluOp),
    /// BIT test (check bit N, set flags).
    BitTest(u8),
}

/// What to do after popping 2 bytes from the stack.
#[derive(Debug)]
pub(crate) enum PopAction {
    /// Set a 16-bit register pair.
    SetRegister(Register16),
    /// Set PC (RET). Trailing internal = true.
    SetPc,
    /// Set PC + enable interrupts (RETI). Trailing internal = true.
    SetPcEnableInterrupts,
}

/// Read-modify-write operation on a memory byte.
#[derive(Debug)]
pub(crate) enum RmwOp {
    Increment,
    Decrement,
    Rotate(Direction, Carry),
    ShiftArithmetical(Direction),
    ShiftRightLogical,
    Swap,
    BitSet(u8),
    BitReset(u8),
}

// ── Phase enum ──────────────────────────────────────────────────────────

/// The behavior of the current instruction's post-decode M-cycles,
/// expressed as a sequence of bus actions. The CPU walks through the
/// phase yielding one `MCycleAction` per M-cycle via `next_mcycle()`.
#[derive(Debug)]
#[allow(private_interfaces)]
pub(crate) enum Phase {
    /// Read operand bytes, then decode and transition to the execution
    /// phase. The opcode has already been read in the Fetch CpuPhase.
    Operands {
        pc: u16,
        bytes: [u8; 3],
        bytes_read: u8,
        bytes_needed: u8,
    },

    /// One memory read, then a CPU action.
    ReadOp { address: u16, action: ReadAction },

    /// Read-modify-write on a memory address.
    ReadModifyWrite { address: u16, op: RmwOp },

    /// One memory write.
    WriteOp {
        address: u16,
        value: u8,
        hl_post: i16,
    },

    /// Two memory writes (LD [a16],SP).
    Write16 { address: u16, lo: u8, hi: u8 },

    /// N internal cycles, no bus activity.
    InternalOp { count: u8 },

    /// Single internal cycle where the IDU places an address on the bus.
    InternalOamBug { address: u16 },

    /// Pop: 2 stack reads + optional trailing internal.
    Pop { sp: u16, action: PopAction },

    /// Push: 1 internal + 2 writes (SP decremented incrementally).
    Push { sp: u16, hi: u8, lo: u8 },

    /// Conditional jump: 0 or 1 internal.
    CondJump { taken: bool, target: u16 },

    /// Conditional call: if taken, internal + 2 writes.
    CondCall {
        taken: bool,
        sp: u16,
        hi: u8,
        lo: u8,
    },

    /// Conditional return: internal + (if taken: 2 reads + internal).
    CondReturn {
        taken: bool,
        sp: u16,
        action: PopAction,
    },

    /// Trailing fetch-overlap M-cycle: reads the next instruction's
    /// opcode while the in-flight instruction's `commit` retires at
    /// the closing edge (same edge as `zacw` capture). Phase-changers
    /// (EnterHalt / EnterStop / Invalid) are peeled off at the opening
    /// edge so halt/lockup routing fires before this cell.
    FetchOverlap { commit: Commit },

    /// No post-fetch M-cycles (NOP, LD r,r, ALU A,r, HALT, STOP, etc.).
    Empty,
}

// ── CpuPhase ────────────────────────────────────────────────────────────

/// The CPU's top-level execution phase. The CPU is a persistent state
/// machine that continuously cycles through these phases, yielding one
/// `BusAction` per T-cycle.
#[derive(Debug)]
pub(crate) enum CpuPhase {
    /// Generic fetch: reading opcode at [PC]. First M-cycle of every
    /// instruction, and the last M-cycle of the previous instruction
    /// (fetch/execute overlap on hardware).
    Fetch,

    /// Halted: one of three sub-states on the halt sequencer between the
    /// HALT instruction's retire and dispatch's M1 (or a fetch on
    /// IME=0 wake / HALT-bug). See [`HaltPhase`].
    Halted(HaltPhase),

    /// Fetching operand bytes and/or executing post-decode M-cycles.
    Execute { phase: Phase, step: u8 },

    /// ISR dispatch: 3 post-fetch M-cycles (M1-M3 from research doc).
    /// M0 (the detecting fetch) already happened in Fetch phase.
    InterruptDispatch {
        sp: u16,
        pc_hi: u8,
        pc_lo: u8,
        step: u8,
    },

    /// CPU is hard-locked by an illegal opcode. Hardware continues
    /// to tick; the CPU never resumes and never checks for interrupts.
    Locked,
}

/// Halt-sequencer sub-states. During HALT, `mcyc` is parked at `m7` by
/// the `set_mcyc7_n` force-path; halt release is combinational on
/// `irq_latched.q↑` via `ykua → ynkw`.
#[derive(Debug, Clone, Copy)]
pub(crate) enum HaltPhase {
    /// Steady-state spin. Boundary captures `irq_latched`; on
    /// capture-true the next M-cycle is the m7-driven post-halt fetch
    /// (Fetch on IME=0, WakeIntake on IME=1).
    Spin,
    /// One extra M-cycle so `irq_latched` captures on the next CLK9↑.
    SetupMiss,
    /// IME=1 stand-in for the discarded m7 fetch plus the
    /// `dispatch_active.q` (zacw) capture cycle.
    WakeIntake,
}
