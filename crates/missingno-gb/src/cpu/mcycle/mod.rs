use super::{
    Cpu, HaltState, InterruptMasterEnable,
    commit::Commit,
    instructions::Instruction,
    instructions::Interrupt as InterruptInstruction,
    instructions::bit_shift::{Carry, Direction},
    registers::{Register8, Register16},
};

mod apply;
mod build;

// ── Bus action ──────────────────────────────────────────────────────────

/// What happens on the memory bus during one M-cycle.
#[derive(Debug)]
pub(super) enum MCycleAction {
    /// Read a byte at the given address.
    Read { address: u16 },
    /// Write a byte to the given address.
    Write { address: u16, value: u8 },
    /// No bus activity (internal CPU work). The address stays on the
    /// bus pins from the previous request (hardware cpu_bus_pass).
    Internal { address: u16 },
    /// Internal cycle where the IDU places an address on the bus, potentially
    /// triggering the DMG OAM corruption bug if the address is in 0xFE00-0xFEFF
    /// and the PPU is in Mode 2.
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
pub(super) enum AluOp {
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
enum ReadAction {
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
enum PopAction {
    /// Set a 16-bit register pair.
    SetRegister(Register16),
    /// Set PC (RET). Trailing internal = true.
    SetPc,
    /// Set PC + enable interrupts (RETI). Trailing internal = true.
    SetPcEnableInterrupts,
}

/// Read-modify-write operation on a memory byte.
#[derive(Debug)]
enum RmwOp {
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

/// Returns the number of operand bytes following a given opcode (0, 1, or 2).
fn operand_count(opcode: u8) -> u8 {
    match opcode {
        // 1 operand byte: LD r,d8 / LD [HL],d8
        0x06 | 0x0e | 0x16 | 0x1e | 0x26 | 0x2e | 0x36 | 0x3e => 1,
        // 1 operand byte: ALU A,d8
        0xc6 | 0xce | 0xd6 | 0xde | 0xe6 | 0xee | 0xf6 | 0xfe => 1,
        // 1 operand byte: JR e8, JR cc,e8
        0x18 | 0x20 | 0x28 | 0x30 | 0x38 => 1,
        // 1 operand byte: LDH [a8],A / LDH A,[a8]
        0xe0 | 0xf0 => 1,
        // 1 operand byte: ADD SP,e8 / LD HL,SP+e8
        0xe8 | 0xf8 => 1,
        // 1 operand byte: CB prefix
        0xcb => 1,
        // 1 operand byte: STOP
        0x10 => 1,

        // 2 operand bytes: LD r16,d16
        0x01 | 0x11 | 0x21 | 0x31 => 2,
        // 2 operand bytes: LD [a16],SP
        0x08 => 2,
        // 2 operand bytes: LD [a16],A / LD A,[a16]
        0xea | 0xfa => 2,
        // 2 operand bytes: JP a16, JP cc,a16
        0xc3 | 0xc2 | 0xca | 0xd2 | 0xda => 2,
        // 2 operand bytes: CALL a16, CALL cc,a16
        0xcd | 0xc4 | 0xcc | 0xd4 | 0xdc => 2,

        // Everything else: 0 operand bytes
        _ => 0,
    }
}

// ── CpuPhase ────────────────────────────────────────────────────────────

/// The CPU's top-level execution phase. The CPU is a persistent state
/// machine that continuously cycles through these phases, yielding one
/// `BusAction` per dot.
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

// ── CPU state machine methods ───────────────────────────────────────────

impl Cpu {
    /// Advance one dot (T-cycle). Returns a `BusAction` that the executor
    /// must handle (tick hardware, perform bus operations).
    ///
    /// The CPU is a continuous state machine — this method always returns
    /// a `BusAction`. When an instruction completes, the boundary flag is
    /// set and the first dot of the next instruction is deferred to the
    /// next call.
    pub fn next_tcycle(&mut self) -> BusAction {
        // At the start of a new M-cycle, fetch the next MCycleAction.
        // The CPU always chains into the next M-cycle (enter_fetch chains
        // into mcycle_fetch, etc.), so next_mcycle always returns Some.
        if !self.mcycle_active {
            // Save the previous M-cycle's bus address before replacing.
            // On hardware, op_addr holds the old value until DELTA_EF.
            let action = self
                .next_mcycle()
                .expect("next_mcycle must always return Some (CPU chains at boundaries)");
            self.current_action = Some(action);
            self.tcycle = TCycle::ZERO;
            self.mcycle_active = true;
        }

        let tcycle = self.tcycle;
        self.last_tcycle = tcycle;
        self.tcycle = if tcycle.as_u8() == 3 {
            TCycle::ZERO
        } else {
            tcycle.advance()
        };

        let result = match &self.current_action {
            Some(MCycleAction::Read { address }) => {
                // CPU latches read data at the end of the M-cycle.
                if tcycle.as_u8() == 3 {
                    BusAction::Read { address: *address }
                } else {
                    BusAction::Idle
                }
            }
            Some(MCycleAction::Write { address, value }) => {
                // CPU write commits at the end of the M-cycle.
                if tcycle.as_u8() == 3 {
                    BusAction::Write {
                        address: *address,
                        value: *value,
                    }
                } else {
                    BusAction::Idle
                }
            }
            Some(MCycleAction::InternalOamBug { address }) => {
                // IDU address is on the bus during the first T-cycle.
                if tcycle.as_u8() == 0 {
                    BusAction::InternalOamBug { address: *address }
                } else {
                    BusAction::Idle
                }
            }
            Some(MCycleAction::Internal { .. }) => BusAction::Idle,
            None => unreachable!(),
        };

        if tcycle.as_u8() == 3 {
            self.mcycle_active = false;
            self.boundary_pending = true;
        }

        self.last_bus_action = result;
        result
    }

    /// Pick the next M-cycle's bus action. Single combinational
    /// selector over post-edge state — `irq_latched.q`,
    /// `dispatch_active.q`, and `irq_pending` have all settled when
    /// this runs.
    fn next_mcycle(&mut self) -> Option<MCycleAction> {
        // M_h start: halt-bug-vs-halt-state decision. yoii captured
        // the pre-update_latch dispatch.latched() at this boundary, so
        // IF rises held by the per-bit latch through HALT body's
        // data-phase see the pre-release value here.
        if self.halt.bug_check_pending {
            self.halt.bug_check_pending = false;
            if self.irq.irq_latched.output() {
                // Halt RS-latch can't set (ykua holds reset LOW).
                self.halt.state = HaltState::Running;
                let ime_enabled = self.irq.ime.output() == InterruptMasterEnable::Enabled;
                if ime_enabled {
                    // Collapse HALT-IDU+1 + dispatch's universal -1
                    // step: PC HALT+1 → HALT_addr.
                    self.bus_counter = self.bus_counter.wrapping_sub(1);
                    if self.dispatch.dispatch_active() {
                        let pc = self.bus_counter;
                        self.phase = CpuPhase::InterruptDispatch {
                            sp: self.stack_pointer,
                            pc_hi: (pc >> 8) as u8,
                            pc_lo: (pc & 0xff) as u8,
                            step: 0,
                        };
                        self.exec_step = 0;
                        self.irq.pending_vector_resolve = false;
                        self.boundary_flag = true;
                        return self.mcycle_isr();
                    }
                } else {
                    // HALT-bug: PC++ suppression at the next opcode
                    // fetch makes the byte after HALT execute twice.
                    self.halt.bug = true;
                }
                // Phase is already Execute(FetchOverlap step 1) from
                // enter_fetch_overlap's halt-entry branch; this M-cycle
                // reads HALT+1 (or HALT_addr after pc--).
            } else {
                // No IF pending at M_h start: halt RS-latch sets.
                self.halt.rs_latched = true;
                self.phase = CpuPhase::Halted(HaltPhase::Spin);
                self.exec_step = 0;
            }
        }

        match &self.phase {
            CpuPhase::Fetch => self.mcycle_fetch(),
            CpuPhase::Execute { .. } => self.mcycle_execute(),
            CpuPhase::InterruptDispatch { .. } => self.mcycle_isr(),
            CpuPhase::Halted(HaltPhase::Spin) => {
                if self.irq.irq_latched.output() {
                    let ime_enabled = self.irq.ime.output() == InterruptMasterEnable::Enabled;
                    let dispatch_pending = ime_enabled && !self.dispatch.latched().is_empty();
                    if dispatch_pending {
                        Some(self.mcycle_halted_entry(HaltPhase::WakeIntake))
                    } else {
                        self.enter_post_halt_fetch()
                    }
                } else if self.irq.irq_pending {
                    Some(self.mcycle_halted_entry(HaltPhase::SetupMiss))
                } else {
                    Some(self.mcycle_halted_entry(HaltPhase::Spin))
                }
            }
            CpuPhase::Halted(HaltPhase::SetupMiss) => {
                Some(self.mcycle_halted_entry(HaltPhase::WakeIntake))
            }
            CpuPhase::Halted(HaltPhase::WakeIntake) => {
                // IME=1 dispatch capture: zacw captures `dispatch_active.q = 1`
                // and routes the next M-cycle to dispatch M1. The IME=0
                // fall-through stays defensive for the SetupMiss path; the
                // primary IME=0 wake short-circuits at the Spin arm above.
                let ime_enabled = self.irq.ime.output() == InterruptMasterEnable::Enabled;
                let irq_pending_for_dispatch = ime_enabled && !self.dispatch.latched().is_empty();
                if irq_pending_for_dispatch {
                    self.halt.state = HaltState::Running;
                    self.halt.rs_latched = false;
                    self.halt.wake_active = true;
                    let pc = self.bus_counter;
                    self.phase = CpuPhase::InterruptDispatch {
                        sp: self.stack_pointer,
                        pc_hi: (pc >> 8) as u8,
                        pc_lo: (pc & 0xff) as u8,
                        step: 0,
                    };
                    self.exec_step = 0;
                    self.irq.pending_vector_resolve = false;
                    self.boundary_flag = true;
                    self.mcycle_isr()
                } else {
                    self.enter_post_halt_fetch()
                }
            }
        }
    }

    /// Fetch phase: single M-cycle reading opcode at [PC].
    /// Returns `None` when the fetched instruction has no post-decode
    /// M-cycles (e.g., NOP) — the instruction completes immediately.
    fn mcycle_fetch(&mut self) -> Option<MCycleAction> {
        let step = self.exec_step;
        self.exec_step += 1;

        if step == 0 {
            // Emit the fetch read from bus_counter. Any pending jump
            // target was already consumed in enter_fetch(), which
            // updated bus_counter to the target address.
            Some(MCycleAction::Read {
                address: self.bus_counter,
            })
        } else {
            // Dispatch decision lives inside retire_edge() at the
            // retiring instruction's CLK9↑ — dispatch_active.q gates
            // whether the next M-cycle is dispatch's M1 or a fetch.
            // The HALT-entry transition (Commit::EnterHalt → halt_state
            // = Halting) resolves inside retire_edge's fetch arm,
            // setting phase = Halted(Spin) directly.

            // Normal opcode consumption — set PC from the fetch address.
            // On hardware, reg.pc = bus_addr + 1 at this execute step.
            // If the fetch was from a jump target (different from
            // bus_counter), this is where PC physically updates.
            let opcode = self.data_latch;
            let fetch_addr = match &self.current_action {
                Some(MCycleAction::Read { address }) => *address,
                _ => self.bus_counter,
            };
            if self.halt.bug {
                self.halt.bug = false;
            } else {
                self.bus_counter = fetch_addr.wrapping_add(1);
            }

            let needed = operand_count(opcode);
            if needed == 0 {
                let bytes = [opcode, 0, 0];
                let (instruction, phase, commit) = self.decode_retire(bytes, 1);
                self.instruction = instruction;
                if matches!(phase, Phase::Empty) {
                    Some(self.enter_fetch_overlap(commit))
                } else {
                    // Multi-Mcyc 0-operand op (LD (HL),A, POP rr, etc.):
                    // run its execute phase before fetch overlap.
                    self.phase = CpuPhase::Execute { phase, step: 0 };
                    self.exec_step = 0;
                    self.mcycle_execute()
                }
            } else {
                // Need operand bytes — enter Execute with Operands phase
                self.phase = CpuPhase::Execute {
                    phase: Phase::Operands {
                        pc: self.bus_counter,
                        bytes: [opcode, 0, 0],
                        bytes_read: 1,
                        bytes_needed: 1 + needed,
                    },
                    step: 0,
                };
                self.exec_step = 0;
                self.mcycle_execute()
            }
        }
    }

    /// Enter `CpuPhase::Halted(phase)`. No bus activity — the halted
    /// state holds the address bus passively (dmg-sim shows no
    /// `bus_read` fires in any of the three halt sub-phases).
    fn mcycle_halted_entry(&mut self, phase: HaltPhase) -> MCycleAction {
        self.phase = CpuPhase::Halted(phase);
        self.exec_step = 0;
        self.boundary_flag = true;
        MCycleAction::Internal {
            address: self.bus_counter,
        }
    }

    /// Drop halt and start the post-halt opcode fetch on the IME=0 wake
    /// path. With `mcyc = m7` parked through HALT, this M-cycle carries
    /// the m7-driven post-body fetch from PC.
    fn enter_post_halt_fetch(&mut self) -> Option<MCycleAction> {
        self.halt.state = HaltState::Running;
        self.halt.rs_latched = false;
        self.phase = CpuPhase::Fetch;
        self.exec_step = 0;
        self.boundary_flag = true;
        self.mcycle_fetch()
    }

    /// Execute phase: operand reading and post-decode M-cycles.
    ///
    /// Returns `None` when the instruction is complete (the CPU has
    /// transitioned to Fetch). Returns `Some(action)` for in-progress
    /// M-cycles.
    ///
    /// Uses `std::mem::replace` to take the phase out, avoiding
    /// simultaneous borrows of `self.phase` and `&mut self`.
    fn mcycle_execute(&mut self) -> Option<MCycleAction> {
        // Take the phase out to avoid borrow conflicts.
        let taken = std::mem::replace(&mut self.phase, CpuPhase::Fetch);
        let (mut phase, mut step) = match taken {
            CpuPhase::Execute { phase, step } => (phase, step),
            _ => unreachable!("mcycle_execute called outside Execute phase"),
        };

        let current_step = step;
        step += 1;

        let (action, put_back) = self.execute_phase_step(&mut phase, current_step);

        if put_back {
            self.phase = CpuPhase::Execute { phase, step };
        }

        action
    }

    /// Process one M-cycle step of the Execute phase. Returns `(action, put_back)`:
    /// - `Some(action)` for an in-progress M-cycle, `None` for instruction completion.
    /// - `put_back = true` means the phase should be stored back in self.phase.
    fn execute_phase_step(
        &mut self,
        phase: &mut Phase,
        current_step: u8,
    ) -> (Option<MCycleAction>, bool) {
        match phase {
            Phase::Operands {
                pc,
                bytes,
                bytes_read,
                bytes_needed,
            } => {
                if current_step == 0 && *bytes_read < *bytes_needed {
                    return (Some(MCycleAction::Read { address: *pc }), true);
                }

                // Operand byte just read
                bytes[*bytes_read as usize] = self.data_latch;
                *bytes_read += 1;
                *pc = pc.wrapping_add(1);
                self.bus_counter = *pc;

                if *bytes_read >= *bytes_needed {
                    // Last operand byte. On hardware, JP nn uses bus_pass
                    // (not bus_read) after the last operand, so reg.pc does
                    // NOT advance past the operand. JR uses bus_read for the
                    // operand, and taken JR's build phase immediately
                    // overwrites pc with the target. Not-taken JR/JP cc must
                    // advance pc normally to point past the operand.
                    let opcode = bytes[0];
                    let is_jp_nn = matches!(
                        opcode,
                        0xC3 | 0xC2 | 0xCA | 0xD2 | 0xDA // JP nn / JP cc,nn
                    );
                    if !is_jp_nn {
                    }
                    let b = *bytes;
                    let n = *bytes_read;
                    let (instruction, phase, commit) = self.decode_retire(b, n);
                    self.instruction = instruction;
                    if matches!(phase, Phase::Empty) {
                        return (Some(self.enter_fetch_overlap(commit)), false);
                    }
                    self.phase = CpuPhase::Execute { phase, step: 0 };
                    self.exec_step = 0;
                    return (self.next_mcycle(), false);
                }

                // Non-last operand: issue bus_read for next byte.
                // On hardware, reg.pc = adp fires with cpu_bus_read.
                (Some(MCycleAction::Read { address: *pc }), true)
            }

            Phase::Empty => {
                unreachable!(
                    "Phase::Empty routes through enter_fetch_overlap; never enters Execute"
                )
            }

            Phase::FetchOverlap { commit } => {
                debug_assert!(
                    current_step == 1,
                    "FetchOverlap step 0 is performed inline by enter_fetch_overlap"
                );

                let carried = std::mem::replace(commit, Commit::NoOperation);
                Self::apply_commit(self, carried);

                let opcode = self.data_latch;
                let fetch_addr = match &self.current_action {
                    Some(MCycleAction::Read { address }) => *address,
                    _ => self.bus_counter,
                };

                // dispatch_active.q captured HIGH at this M-cycle's
                // closing CLK9↑ when zaij asserted during the M-cycle's
                // dot 3 eval — i.e. ctl_fetch was high (FetchOverlap is
                // the m6 / m7 fetch state in netlist terms). The dispatch
                // saves PC = fetch_addr (the address of the just-fetched
                // opcode = address of the next instruction), so RETI
                // resumes at the prefetched-then-discarded instruction.
                // Per the netlist: dispatch's M5 vector fetch overwrites
                // IR, and the PC DFF captures the same fetch_addr at this
                // edge (no PC++ to fetch_addr+1 since dispatch's M1 is
                // internal — the would-be PC commit is overridden).
                if self.dispatch.dispatch_active() {
                    let pc = fetch_addr;
                    self.phase = CpuPhase::InterruptDispatch {
                        sp: self.stack_pointer,
                        pc_hi: (pc >> 8) as u8,
                        pc_lo: (pc & 0xff) as u8,
                        step: 0,
                    };
                    self.exec_step = 0;
                    self.irq.pending_vector_resolve = false;
                    self.boundary_flag = true;
                    self.bus_counter = pc;
                    return (self.next_mcycle(), false);
                }

                if self.halt.bug {
                    self.halt.bug = false;
                } else {
                    self.bus_counter = fetch_addr.wrapping_add(1);
                }

                let needed = operand_count(opcode);
                if needed == 0 {
                    let bytes = [opcode, 0, 0];
                    let (instruction, next_phase, next_commit) = self.decode_retire(bytes, 1);
                    self.instruction = instruction;
                    if matches!(next_phase, Phase::Empty) {
                        (Some(self.enter_fetch_overlap(next_commit)), false)
                    } else {
                        self.phase = CpuPhase::Execute {
                            phase: next_phase,
                            step: 0,
                        };
                        self.exec_step = 0;
                        (self.next_mcycle(), false)
                    }
                } else {
                    self.phase = CpuPhase::Execute {
                        phase: Phase::Operands {
                            pc: self.bus_counter,
                            bytes: [opcode, 0, 0],
                            bytes_read: 1,
                            bytes_needed: 1 + needed,
                        },
                        step: 0,
                    };
                    self.exec_step = 0;
                    (self.next_mcycle(), false)
                }
            }

            Phase::ReadOp { address, action } => match current_step {
                0 => (Some(MCycleAction::Read { address: *address }), true),
                _ => {
                    Self::apply_read_action(self, action, self.data_latch);
                    (Some(self.enter_fetch_overlap(Commit::NoOperation)), false)
                }
            },

            Phase::ReadModifyWrite { address, op } => {
                let address = *address;
                match current_step {
                    0 => (Some(MCycleAction::Read { address }), true),
                    1 => {
                        let result = Self::apply_rmw(self, op, self.data_latch);
                        (
                            Some(MCycleAction::Write {
                                address,
                                value: result,
                            }),
                            true,
                        )
                    }
                    _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
                }
            }

            Phase::WriteOp {
                address,
                value,
                hl_post,
            } => match current_step {
                0 => {
                    if *hl_post != 0 {
                        let hl = self.get_register16(Register16::Hl);
                        self.set_register16(Register16::Hl, hl.wrapping_add(*hl_post as u16));
                    }
                    (
                        Some(MCycleAction::Write {
                            address: *address,
                            value: *value,
                        }),
                        true,
                    )
                }
                _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
            },

            Phase::Write16 { address, lo, hi } => {
                let address = *address;
                match current_step {
                    0 => (
                        Some(MCycleAction::Write {
                            address,
                            value: *lo,
                        }),
                        true,
                    ),
                    1 => (
                        Some(MCycleAction::Write {
                            address: address.wrapping_add(1),
                            value: *hi,
                        }),
                        true,
                    ),
                    _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
                }
            }

            Phase::InternalOp { count } => {
                if current_step < *count {
                    (Some(MCycleAction::Internal { address: self.bus_counter }), true)
                } else {
                    (Some(self.enter_fetch_overlap(Commit::NoOperation)), false)
                }
            }

            Phase::InternalOamBug { address } => match current_step {
                0 => (Some(MCycleAction::InternalOamBug { address: *address }), true),
                _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
            },

            Phase::Pop { sp, action } => {
                let sp = *sp;
                match current_step {
                    0 => (Some(MCycleAction::Read { address: sp }), true),
                    1 => {
                        self.scratch = self.data_latch;
                        (
                            Some(MCycleAction::Read {
                                address: sp.wrapping_add(1),
                            }),
                            true,
                        )
                    }
                    2 => {
                        Self::apply_pop(self, action, self.scratch, self.data_latch, sp);
                        let has_trailing =
                            matches!(action, PopAction::SetPc | PopAction::SetPcEnableInterrupts);
                        if has_trailing {
                            (Some(MCycleAction::Internal { address: self.bus_counter }), true)
                        } else {
                            (Some(self.enter_fetch_overlap(Commit::NoOperation)), false)
                        }
                    }
                    _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
                }
            }

            Phase::Push { sp, hi, lo } => {
                let sp = *sp;
                match current_step {
                    0 => (Some(MCycleAction::InternalOamBug { address: sp }), true),
                    1 => {
                        let addr = sp.wrapping_sub(1);
                        self.stack_pointer = addr;
                        (
                            Some(MCycleAction::Write {
                                address: addr,
                                value: *hi,
                            }),
                            true,
                        )
                    }
                    2 => {
                        let addr = sp.wrapping_sub(2);
                        self.stack_pointer = addr;
                        (
                            Some(MCycleAction::Write {
                                address: addr,
                                value: *lo,
                            }),
                            true,
                        )
                    }
                    _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
                }
            }

            Phase::CondJump { taken, target } => {
                if current_step == 0 && *taken {
                    // Internal M-cycle: store the jump target for the
                    // next fetch. On hardware, the PC register stays at
                    // the post-operand address during this M-cycle. The
                    // target is placed on the bus at DELTA_EF, and PC
                    // updates to target+1 when the fetch processes.
                    self.pending_jump_target = Some(*target);
                    (Some(MCycleAction::Internal { address: self.bus_counter }), true)
                } else {
                    if let Some(target) = self.pending_jump_target.take() {
                        self.bus_counter = target;
                    }
                    (Some(self.enter_fetch_overlap(Commit::NoOperation)), false)
                }
            }

            Phase::CondCall { taken, sp, hi, lo } => {
                if !*taken {
                    return (Some(self.enter_fetch_overlap(Commit::NoOperation)), false);
                }
                let sp = *sp;
                match current_step {
                    0 => (Some(MCycleAction::InternalOamBug { address: sp }), true),
                    1 => {
                        let addr = sp.wrapping_sub(1);
                        self.stack_pointer = addr;
                        (
                            Some(MCycleAction::Write {
                                address: addr,
                                value: *hi,
                            }),
                            true,
                        )
                    }
                    2 => {
                        let addr = sp.wrapping_sub(2);
                        self.stack_pointer = addr;
                        (
                            Some(MCycleAction::Write {
                                address: addr,
                                value: *lo,
                            }),
                            true,
                        )
                    }
                    _ => {
                        if let Some(target) = self.pending_jump_target.take() {
                            self.bus_counter = target;
                        }
                        (Some(self.enter_fetch_overlap(Commit::NoOperation)), false)
                    }
                }
            }

            Phase::CondReturn { taken, sp, action } => {
                let sp = *sp;
                let taken = *taken;
                match current_step {
                    0 => (Some(MCycleAction::Internal { address: self.bus_counter }), true),
                    1 if !taken => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
                    1 => (Some(MCycleAction::Read { address: sp }), true),
                    2 => {
                        self.scratch = self.data_latch;
                        (
                            Some(MCycleAction::Read {
                                address: sp.wrapping_add(1),
                            }),
                            true,
                        )
                    }
                    3 => {
                        Self::apply_pop(self, action, self.scratch, self.data_latch, sp);
                        (Some(MCycleAction::Internal { address: self.bus_counter }), true)
                    }
                    _ => (Some(self.enter_fetch_overlap(Commit::NoOperation)), false),
                }
            }
        }
    }

    /// ISR dispatch: 5 M-cycles (steps 0..=4) per gb-ctr RST n p129.
    ///   step 0 → M1 internal (PC on bus)
    ///   step 1 → M2 InternalOamBug(SP)
    ///   step 2 → M3 push pc_hi (Write {sp-1})
    ///   step 3 → M4 push pc_lo (Write {sp-2}); vector resolved here
    ///   step 4 → M5 vector fetch (via enter_fetch_overlap)
    /// IME (zacw downstream) clears on the dispatching CLK9↑ — step 0's
    /// `write_immediate(Disabled)` on both stages.
    fn mcycle_isr(&mut self) -> Option<MCycleAction> {
        let (sp, pc_hi, pc_lo, step) = match &mut self.phase {
            CpuPhase::InterruptDispatch {
                sp,
                pc_hi,
                pc_lo,
                step,
            } => (*sp, *pc_hi, *pc_lo, step),
            _ => unreachable!("mcycle_isr called outside InterruptDispatch phase"),
        };

        let current_step = *step;
        *step += 1;

        match current_step {
            // M1: IDU PC- — on hardware this undoes the wakeup NOP's PC
            // increment. The emulator skips both the increment and decrement
            // for the same net effect. IME clears at the M1 boundary (zacw).
            // Both stages must clear so the boundary copy doesn't restore
            // IME on the next M-cycle.
            0 => {
                self.irq.ime.write_immediate(InterruptMasterEnable::Disabled);
                self.irq.ime_delay = false;
                Some(MCycleAction::Internal { address: self.bus_counter })
            }
            1 => Some(MCycleAction::InternalOamBug { address: sp }),
            2 => {
                let addr = sp.wrapping_sub(1);
                self.stack_pointer = addr;
                Some(MCycleAction::Write {
                    address: addr,
                    value: pc_hi,
                })
            }
            3 => {
                // IE push bug: the vector must be resolved after the
                // high-byte push (step 2) but before this low-byte push.
                self.irq.pending_vector_resolve = true;
                let addr = sp.wrapping_sub(2);
                self.stack_pointer = addr;
                Some(MCycleAction::Write {
                    address: addr,
                    value: pc_lo,
                })
            }
            4 => {
                // ISR complete — trailing fetch overlap reads the handler's first opcode.
                Some(self.enter_fetch_overlap(Commit::NoOperation))
            }
            _ => unreachable!(),
        }
    }

    /// Pure decode — returns the decoded Instruction with its Phase and
    /// retire-edge Commit. Does not mutate IME/dispatch state.
    /// `retire_edge` owns those mutations so `dispatch_trigger`'s snapshot
    /// and the EI/DI dispatch_active-chain gate stay coherent.
    fn decode_retire(&mut self, bytes: [u8; 3], bytes_read: u8) -> (Instruction, Phase, Commit) {
        let mut iter = bytes[..bytes_read as usize].iter().copied();
        let instruction = Instruction::decode(&mut iter).unwrap();

        let (phase, commit) = match &instruction {
            Instruction::Interrupt(InterruptInstruction::Await) => {
                (Phase::Empty, Commit::EnterHalt)
            }
            Instruction::Stop => (Phase::Empty, Commit::EnterStop),
            Instruction::Invalid(_) => (Phase::Empty, Commit::Invalid),
            Instruction::NoOperation => (Phase::Empty, Commit::NoOperation),
            Instruction::DecimalAdjustAccumulator => (Phase::Empty, Commit::Daa),
            Instruction::CarryFlag(cf) => (Phase::Empty, Commit::CarryFlag(cf.clone())),
            Instruction::Interrupt(InterruptInstruction::Enable) => {
                (Phase::Empty, Commit::EnableInterrupts)
            }
            Instruction::Interrupt(InterruptInstruction::Disable) => {
                (Phase::Empty, Commit::DisableInterrupts)
            }

            Instruction::Load(load) => Self::build_load(self, load),
            Instruction::Arithmetic(arith) => Self::build_arithmetic(self, arith),
            Instruction::Bitwise(bw) => Self::build_bitwise(self, bw),
            Instruction::BitShift(bs) => Self::build_bit_shift(self, bs),
            Instruction::BitFlag(bf) => Self::build_bit_flag(self, bf),
            Instruction::Jump(j) => Self::build_jump(self, j),
            Instruction::Stack(s) => Self::build_stack(self, s),
        };

        (instruction, phase, commit)
    }

    /// Enter the trailing fetch-overlap M-cycle at the opening edge.
    /// Captures `zacw` (dispatch_active), routes early to dispatch /
    /// halt when needed. All commits apply inline here so the new
    /// register/flag values are visible at the start of the M-cycle
    /// that fetches the next opcode (gb-ctr's M3/M1 column).
    fn enter_fetch_overlap(&mut self, commit: Commit) -> MCycleAction {
        Self::apply_commit(self, commit);
        let deferred = Commit::NoOperation;

        if let Some(target) = self.pending_jump_target.take() {
            self.bus_counter = target;
        }
        self.boundary_flag = true;

        if self.dispatch.dispatch_active() {
            // zkog/zloz reset fires at ctl_int_entry_m6 (M3→M4 vector
            // resolve), driven by pending_vector_resolve in execute.rs.
            self.halt.state = HaltState::Running;
            self.halt.rs_latched = false;
            let pc = self.bus_counter;
            self.phase = CpuPhase::InterruptDispatch {
                sp: self.stack_pointer,
                pc_hi: (pc >> 8) as u8,
                pc_lo: (pc & 0xff) as u8,
                step: 0,
            };
            self.exec_step = 0;
            self.irq.pending_vector_resolve = false;
            return self
                .next_mcycle()
                .expect("next_mcycle must return Some after dispatch arm");
        }

        if self.halt.state == HaltState::Halting {
            // Defer the halt-bug-vs-halt-state decision to M_h start
            // (the boundary at the end of HALT's body M-cycle). The
            // body M-cycle reads HALT+1 like any overlap fetch;
            // `data_phase_n` pulses normally (rs_latched stays false)
            // so the per-bit irq_latch gates correctly.
            self.halt.state = HaltState::Halted;
            self.halt.rs_latched = false;
            self.halt.bug_check_pending = true;
        }

        self.phase = CpuPhase::Execute {
            phase: Phase::FetchOverlap { commit: deferred },
            step: 1,
        };
        self.exec_step = 1;
        MCycleAction::Read {
            address: self.bus_counter,
        }
    }

    /// The T-cycle that produced the most recent `BusAction`. Used by
    /// the executor to time per-T-cycle work after `next_tcycle()` runs.
    pub fn last_tcycle(&self) -> TCycle {
        self.last_tcycle
    }

    /// True at the last T-cycle of the current M-cycle, where the
    /// boundary work (timers, DMA, serial, audio, PPU boundary)
    /// completes before the next M-cycle begins.
    pub fn at_mcycle_boundary(&self) -> bool {
        self.last_tcycle.as_u8() == 3
    }

    /// Check and consume the instruction boundary flag. Returns true
    /// if the CPU transitioned to the Fetch phase since the last check.
    pub fn take_instruction_boundary(&mut self) -> bool {
        if self.boundary_flag {
            self.boundary_flag = false;
            true
        } else {
            false
        }
    }

    /// Check if the CPU is at an instruction boundary without consuming it.
    pub fn at_instruction_boundary(&self) -> bool {
        self.boundary_flag
    }

    /// IE push bug: consume the pending vector resolution request.
    pub fn take_pending_vector_resolve(&mut self) -> bool {
        if self.irq.pending_vector_resolve {
            self.irq.pending_vector_resolve = false;
            true
        } else {
            false
        }
    }

    /// Update `irq_pending` from the priority-encoded `IF & IE`.
    /// Combinational, not IME-gated — the IME gate sits in
    /// `dispatch_trigger`. The vector is resolved separately via
    /// `pending_vector_resolve` at the ISR's M3→M4 push.
    pub fn update_interrupt_state(
        &mut self,
        triggered: Option<super::super::interrupts::Interrupt>,
    ) {
        self.irq.irq_pending = triggered.is_some();
    }

    /// Clock `irq_latched` (yoii) on its CLK9 capture edge. Its D
    /// input is the data-phase-gated priority chain output
    /// (`dispatch.latched()`), not raw `irq_pending`. yoii drives
    /// the HALT-release chain.
    pub fn tick_irq_latched(&mut self) {
        self.irq.irq_latched.write(!self.dispatch.latched().is_empty());
        self.irq.irq_latched.tick();
    }
}
