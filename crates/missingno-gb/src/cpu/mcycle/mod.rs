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
pub(super) enum BusAction {
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

// ── Bus dot (ring counter phase model) ────────────────────────────────

/// The CPU bus timing signals for the current dot within an M-cycle.
///
/// In hardware, AFUR/ALEF/APUK/ADYK form a 4-DFF ring counter producing
/// 8 phases (A-H) per M-cycle. Each emulator dot spans 2 phases. The
/// named signals here are the same combinational outputs that hardware
/// derives from the ring counter DFF states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusDot(u8);

impl BusDot {
    /// Dot 0 (phases A,B). First dot of the M-cycle.
    pub const ZERO: BusDot = BusDot(0);
    /// Dot 1 (phases C,D). Second dot of the M-cycle.
    pub const ONE: BusDot = BusDot(1);

    /// Raw dot number (0–3) for trace output.
    pub fn as_u8(self) -> u8 {
        self.0
    }

    pub fn advance(self) -> BusDot {
        debug_assert!(self.0 < 3, "cannot advance past dot 3");
        BusDot(self.0 + 1)
    }

    /// BOGA_Axxxxxxx: M-cycle boundary. Active during phase A only.
    /// Rising edge at H->A marks the transition between M-cycles.
    ///
    /// In the emulator's dot model, this fires at dot 3 because the
    /// M-cycle boundary work (interrupt latch, DMA, serial, audio)
    /// must complete before the next M-cycle's dot 0 begins.
    pub fn boga(self) -> bool {
        self.0 == 3
    }

    /// BOWA_Axxxxxxx: Address latch clock. The CPU places the address
    /// on the bus during phase A.
    ///
    /// Used for: OAM bug address recording (IDU address on bus).
    pub fn bowa(self) -> bool {
        self.0 == 0
    }

    /// BUDE_xxxxEFGH: Write data window. The CPU drives write data
    /// onto the bus during phases E-H (dots 2-3).
    ///
    /// MOPA_xxxxEFGH: Second half of the M-cycle. Rising edge at
    /// D->E (start of dot 2).
    ///
    /// Used for: OAM bug fire timing (CUFE_OAM_CLKp fires when
    /// MOPA goes high while SARO_ADDR_OAMp is active).
    pub fn mopa(self) -> bool {
        self.0 >= 2
    }

    /// AFAS_xxxxEFGx: Write pulse window. Active during phases E,F,G.
    /// Falling edge at G->H is the DFF latch point for register writes.
    ///
    /// Used for: Write action placement (the actual bus write that
    /// latches at the G->H boundary = end of dot 3).
    pub fn afas_falling(self) -> bool {
        self.0 == 3
    }

    /// BUKE_AxxxxxGH: Data latch window. The data latch accumulates
    /// bus data during phases G,H,A. CPU reads the latch at H->A.
    ///
    /// Used for: Read action placement (data capture at end of
    /// M-cycle, coinciding with BOGA).
    pub fn buke(self) -> bool {
        self.0 == 0 || self.0 == 3
    }

    /// Raw dot index (0-3). Escape hatch for the rare cases where
    /// a named signal doesn't exist (e.g., fetch dot counter).
    /// Prefer named signals in all other contexts.
    pub fn index(self) -> u8 {
        self.0
    }
}

// ── Dot-level bus state ─────────────────────────────────────────────────

/// What the CPU bus is doing during one dot (T-cycle).
///
/// The executor ticks all hardware and routes these to subsystems.
/// Each M-cycle expands into 4 dots with bus operations placed at
/// the hardware-correct position:
/// - **Read**:     `[Idle, Idle, Idle, Read]`
/// - **Write**:    `[Idle, Idle, Idle, Write]`
/// - **Internal**: `[Idle, Idle, Idle, Idle]`
/// - **OamBug**:   `[InternalOamBug, Idle, Idle, Idle]`
#[derive(Clone, Copy, Debug)]
pub enum DotAction {
    /// No bus transfer this dot.
    Idle,
    /// CPU is reading from this address. The executor must provide
    /// the value before the next M-cycle begins.
    Read { address: u16 },
    /// CPU is writing this value to this address. The write latches
    /// at this dot (G→H boundary, end of M-cycle).
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
/// phase yielding one `BusAction` per M-cycle via `next_mcycle()`.
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
/// `DotAction` per dot.
#[derive(Debug)]
pub(crate) enum CpuPhase {
    /// Generic fetch: reading opcode at [PC]. First M-cycle of every
    /// instruction, and the last M-cycle of the previous instruction
    /// (fetch/execute overlap on hardware).
    Fetch,

    /// Halted: spinning in fetch-like reads at [PC] without incrementing.
    /// Exits to Execute (IME=0 wakeup) or InterruptDispatch (IME=1 wakeup).
    Halted,

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

// ── CPU state machine methods ───────────────────────────────────────────

impl Cpu {
    /// Advance one dot (T-cycle). Returns a `DotAction` that the executor
    /// must handle (tick hardware, perform bus operations).
    ///
    /// The CPU is a continuous state machine — this method always returns
    /// a `DotAction`. When an instruction completes, the boundary flag is
    /// set and the first dot of the next instruction is deferred to the
    /// next call.
    pub fn next_dot(&mut self, read_value: u8) -> DotAction {
        // At the start of a new M-cycle, fetch the next BusAction.
        // The CPU always chains into the next M-cycle (enter_fetch chains
        // into mcycle_fetch, etc.), so next_mcycle always returns Some.
        if !self.mcycle_active {
            self.op_state = self.op_state.wrapping_add(1);
            // Save the previous M-cycle's bus address before replacing.
            // On hardware, op_addr holds the old value until DELTA_EF.
            let action = self
                .next_mcycle(read_value)
                .expect("next_mcycle must always return Some (CPU chains at boundaries)");
            self.current_action = Some(action);
            self.dot = BusDot::ZERO;
            self.mcycle_active = true;
        }

        let dot = self.dot;
        self.last_dot = dot;
        self.dot = if dot.boga() {
            BusDot::ZERO
        } else {
            dot.advance()
        };

        let result = match &self.current_action {
            Some(BusAction::Read { address }) => {
                if dot.boga() {
                    DotAction::Read { address: *address }
                } else {
                    DotAction::Idle
                }
            }
            Some(BusAction::Write { address, value }) => {
                if dot.afas_falling() {
                    DotAction::Write {
                        address: *address,
                        value: *value,
                    }
                } else {
                    DotAction::Idle
                }
            }
            Some(BusAction::InternalOamBug { address }) => {
                if dot.bowa() {
                    DotAction::InternalOamBug { address: *address }
                } else {
                    DotAction::Idle
                }
            }
            Some(BusAction::Internal { .. }) => DotAction::Idle,
            None => unreachable!(),
        };

        if dot.boga() {
            self.mcycle_active = false;
        }

        result
    }

    /// Get the next M-cycle's bus action. Returns `None` at an
    /// instruction boundary (the CPU has entered Fetch but the fetch
    /// M-cycle should be deferred to the next `next_dot` call).
    fn next_mcycle(&mut self, read_value: u8) -> Option<BusAction> {
        match self.phase_tag() {
            PhaseTag::Fetch => self.mcycle_fetch(read_value),
            PhaseTag::Halted => self.mcycle_halted(read_value),
            PhaseTag::Execute => self.mcycle_execute(read_value),
            PhaseTag::InterruptDispatch => self.mcycle_isr(read_value),
        }
    }

    fn phase_tag(&self) -> PhaseTag {
        match &self.phase {
            CpuPhase::Fetch => PhaseTag::Fetch,
            CpuPhase::Halted => PhaseTag::Halted,
            CpuPhase::Execute { .. } => PhaseTag::Execute,
            CpuPhase::InterruptDispatch { .. } => PhaseTag::InterruptDispatch,
        }
    }

    /// Fetch phase: single M-cycle reading opcode at [PC].
    /// Returns `None` when the fetched instruction has no post-decode
    /// M-cycles (e.g., NOP) — the instruction completes immediately.
    fn mcycle_fetch(&mut self, read_value: u8) -> Option<BusAction> {
        let step = self.exec_step;
        self.exec_step += 1;

        if step == 0 {
            // Emit the fetch read from bus_counter. Any pending jump
            // target was already consumed in enter_fetch(), which
            // updated bus_counter to the target address.
            Some(BusAction::Read {
                address: self.bus_counter,
            })
        } else {
            // Opcode received — check for HALT entry first
            if self.halt_state == HaltState::Halting {
                // HALT was decoded in the previous instruction. The
                // fetch we just completed was the dummy fetch (read [PC]
                // without incrementing). Transition to Halted.
                self.halt_state = HaltState::Halted;
                self.phase = CpuPhase::Halted;
                self.exec_step = 0;
                // Signal boundary and chain into the first halted NOP.
                self.boundary_flag = true;
                self.instruction_pc = self.bus_counter;
                return self.mcycle_halted(0);
            }

            // Dispatch decision lives inside retire_edge() at the
            // retiring instruction's CLK9↑ — int_entry.q gates whether
            // the next M-cycle is dispatch's M1 or a speculative fetch.
            // By the time this step 1 runs, that decision has landed.

            // Normal opcode consumption — set PC from the fetch address.
            // On hardware, reg.pc = bus_addr + 1 at this execute step.
            // If the fetch was from a jump target (different from
            // bus_counter), this is where PC physically updates.
            let opcode = read_value;
            let fetch_addr = match &self.current_action {
                Some(BusAction::Read { address }) => *address,
                _ => self.bus_counter,
            };
            if self.halt_bug {
                self.halt_bug = false;
            } else {
                self.bus_counter = fetch_addr.wrapping_add(1);
                self.pc = self.bus_counter;
            }

            let needed = operand_count(opcode);
            if needed == 0 {
                // No operands — retire immediately. decode_retire
                // produces the Phase + Commit; retire_edge owns the
                // apply, int_entry capture, and dispatch decision.
                let bytes = [opcode, 0, 0];
                let (instruction, phase, commit) = self.decode_retire(bytes, 1);
                self.instruction = instruction;
                Some(self.retire_edge(commit, phase))
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
                self.mcycle_execute(0)
            }
        }
    }

    /// Halted phase: one HALT idle M-cycle.
    ///
    /// Each call is exactly one M-cycle. Halt release is gated by g42.q
    /// alone (g43 OAI21 collapse: `NOT((yolu OR g42.q) AND reset_n)`);
    /// the ISR-vs-Fetch decision is deferred to the boundary CLK9↑ via
    /// `pending_halt_wake_dispatch`, where `halt_wake_edge` chooses
    /// dispatch vs running fetch.
    fn mcycle_halted(&mut self, _read_value: u8) -> Option<BusAction> {
        // ── Boundary housekeeping ──
        self.exec_step = 0;
        self.boundary_flag = true;
        self.instruction_pc = self.bus_counter;

        // ── Halt release ──
        // Per spec §13.2, g43 has no `int_pending` or `IME` input — the
        // halt-release condition reduces to `g42.output()`. Drop halt
        // unconditionally on g42↑; defer the ISR-vs-Fetch choice to the
        // matching fall via `pending_halt_wake_dispatch`.
        if self.g42.output() {
            self.halt_state = HaltState::Running;
            self.pending_halt_wake_dispatch = true;
            self.phase = CpuPhase::Fetch;
            self.exec_step = 0;
            return Some(BusAction::Read {
                address: self.bus_counter,
            });
        }

        // ── Still halted ──
        Some(BusAction::Read {
            address: self.bus_counter,
        })
    }

    /// Execute phase: operand reading and post-decode M-cycles.
    ///
    /// Returns `None` when the instruction is complete (the CPU has
    /// transitioned to Fetch). Returns `Some(action)` for in-progress
    /// M-cycles.
    ///
    /// Uses `std::mem::replace` to take the phase out, avoiding
    /// simultaneous borrows of `self.phase` and `&mut self`.
    fn mcycle_execute(&mut self, read_value: u8) -> Option<BusAction> {
        // Take the phase out to avoid borrow conflicts.
        let taken = std::mem::replace(&mut self.phase, CpuPhase::Fetch);
        let (mut phase, mut step) = match taken {
            CpuPhase::Execute { phase, step } => (phase, step),
            _ => unreachable!("mcycle_execute called outside Execute phase"),
        };

        let current_step = step;
        step += 1;

        let (action, put_back) = self.execute_phase_step(&mut phase, current_step, read_value);

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
        read_value: u8,
    ) -> (Option<BusAction>, bool) {
        match phase {
            Phase::Operands {
                pc,
                bytes,
                bytes_read,
                bytes_needed,
            } => {
                if current_step == 0 && *bytes_read < *bytes_needed {
                    return (Some(BusAction::Read { address: *pc }), true);
                }

                // Operand byte just read
                bytes[*bytes_read as usize] = read_value;
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
                        self.pc = self.bus_counter;
                    }
                    let b = *bytes;
                    let n = *bytes_read;
                    let (instruction, phase, commit) = self.decode_retire(b, n);
                    self.instruction = instruction;
                    return (Some(self.retire_edge(commit, phase)), false);
                }

                // Non-last operand: issue bus_read for next byte.
                // On hardware, reg.pc = adp fires with cpu_bus_read.
                self.pc = self.bus_counter;
                (Some(BusAction::Read { address: *pc }), true)
            }

            Phase::Empty => (
                Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                false,
            ),

            Phase::ReadOp { address, action } => match current_step {
                0 => (Some(BusAction::Read { address: *address }), true),
                _ => {
                    Self::apply_read_action(self, action, read_value);
                    (
                        Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                        false,
                    )
                }
            },

            Phase::ReadModifyWrite { address, op } => {
                let address = *address;
                match current_step {
                    0 => (Some(BusAction::Read { address }), true),
                    1 => {
                        let result = Self::apply_rmw(self, op, read_value);
                        (
                            Some(BusAction::Write {
                                address,
                                value: result,
                            }),
                            true,
                        )
                    }
                    _ => (
                        Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                        false,
                    ),
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
                        Some(BusAction::Write {
                            address: *address,
                            value: *value,
                        }),
                        true,
                    )
                }
                _ => (
                    Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                    false,
                ),
            },

            Phase::Write16 { address, lo, hi } => {
                let address = *address;
                match current_step {
                    0 => (
                        Some(BusAction::Write {
                            address,
                            value: *lo,
                        }),
                        true,
                    ),
                    1 => (
                        Some(BusAction::Write {
                            address: address.wrapping_add(1),
                            value: *hi,
                        }),
                        true,
                    ),
                    _ => (
                        Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                        false,
                    ),
                }
            }

            Phase::InternalOp { count } => {
                if current_step < *count {
                    (Some(BusAction::Internal { address: self.pc }), true)
                } else {
                    (
                        Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                        false,
                    )
                }
            }

            Phase::InternalOamBug { address } => match current_step {
                0 => (Some(BusAction::InternalOamBug { address: *address }), true),
                _ => (
                    Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                    false,
                ),
            },

            Phase::Pop { sp, action } => {
                let sp = *sp;
                match current_step {
                    0 => (Some(BusAction::Read { address: sp }), true),
                    1 => {
                        self.scratch = read_value;
                        (
                            Some(BusAction::Read {
                                address: sp.wrapping_add(1),
                            }),
                            true,
                        )
                    }
                    2 => {
                        Self::apply_pop(self, action, self.scratch, read_value, sp);
                        let has_trailing =
                            matches!(action, PopAction::SetPc | PopAction::SetPcEnableInterrupts);
                        if has_trailing {
                            (Some(BusAction::Internal { address: self.pc }), true)
                        } else {
                            (
                                Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                                false,
                            )
                        }
                    }
                    _ => (
                        Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                        false,
                    ),
                }
            }

            Phase::Push { sp, hi, lo } => {
                let sp = *sp;
                match current_step {
                    0 => (Some(BusAction::InternalOamBug { address: sp }), true),
                    1 => {
                        let addr = sp.wrapping_sub(1);
                        self.stack_pointer = addr;
                        (
                            Some(BusAction::Write {
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
                            Some(BusAction::Write {
                                address: addr,
                                value: *lo,
                            }),
                            true,
                        )
                    }
                    _ => (
                        Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                        false,
                    ),
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
                    (Some(BusAction::Internal { address: self.pc }), true)
                } else {
                    (
                        Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                        false,
                    )
                }
            }

            Phase::CondCall { taken, sp, hi, lo } => {
                if !*taken {
                    return (
                        Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                        false,
                    );
                }
                let sp = *sp;
                match current_step {
                    0 => (Some(BusAction::InternalOamBug { address: sp }), true),
                    1 => {
                        let addr = sp.wrapping_sub(1);
                        self.stack_pointer = addr;
                        (
                            Some(BusAction::Write {
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
                            Some(BusAction::Write {
                                address: addr,
                                value: *lo,
                            }),
                            true,
                        )
                    }
                    _ => (
                        Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                        false,
                    ),
                }
            }

            Phase::CondReturn { taken, sp, action } => {
                let sp = *sp;
                let taken = *taken;
                match current_step {
                    0 => (Some(BusAction::Internal { address: self.pc }), true),
                    1 if !taken => (
                        Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                        false,
                    ),
                    1 => (Some(BusAction::Read { address: sp }), true),
                    2 => {
                        self.scratch = read_value;
                        (
                            Some(BusAction::Read {
                                address: sp.wrapping_add(1),
                            }),
                            true,
                        )
                    }
                    3 => {
                        Self::apply_pop(self, action, self.scratch, read_value, sp);
                        (Some(BusAction::Internal { address: self.pc }), true)
                    }
                    _ => (
                        Some(self.retire_edge(Commit::NoOperation, Phase::Empty)),
                        false,
                    ),
                }
            }
        }
    }

    /// ISR dispatch: 5 held M-cycles spanning steps 0..=4.
    ///
    /// - Steps 0..=3 each return their own BusAction directly,
    ///   held for one M-cycle by next_dot's bus-output loop.
    /// - Step 4 transitions to Fetch via retire_edge(NoOperation,
    ///   Phase::Empty); the chained mcycle_fetch(0) returns
    ///   Read{vector} as step 4's held M-cycle (M5 vector fetch).
    ///   The handler's first opcode then decodes via the standard
    ///   fetch/execute overlap on the next M-cycle.
    ///
    /// Hardware mapping (gb-ctr §6.7 RST n p129):
    ///   step 0 → M1 internal (Internal{pc})
    ///   step 1 → M2 internal (InternalOamBug{sp})
    ///   step 2 → M3 push pc_hi (Write{sp-1, pc_hi})
    ///   step 3 → M4 push pc_lo (Write{sp-2, pc_lo}, vector resolved here)
    ///   step 4 → M5 vector fetch (Read{vector} via retire_edge chain)
    ///
    /// IME (zacw downstream) is cleared on the dispatching CLK9↑ inside
    /// step 0's `write_immediate(Disabled)`. retire_edge tail-calls
    /// mcycle_isr(0) for the running-CPU dispatch entry, and
    /// halt_wake_edge writes IME directly before producing the same
    /// step-0 effect for the HALT-wake path.
    fn mcycle_isr(&mut self, _read_value: u8) -> Option<BusAction> {
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
            0 => {
                self.ime.write_immediate(InterruptMasterEnable::Disabled);
                Some(BusAction::Internal { address: self.pc })
            }
            1 => Some(BusAction::InternalOamBug { address: sp }),
            2 => {
                let addr = sp.wrapping_sub(1);
                self.stack_pointer = addr;
                Some(BusAction::Write {
                    address: addr,
                    value: pc_hi,
                })
            }
            3 => {
                // IE push bug: the vector must be resolved after the
                // high-byte push (step 2) but before this low-byte push.
                self.pending_vector_resolve = true;
                let addr = sp.wrapping_sub(2);
                self.stack_pointer = addr;
                Some(BusAction::Write {
                    address: addr,
                    value: pc_lo,
                })
            }
            4 => {
                // ISR complete — transition to Fetch at the vector address.
                Some(self.retire_edge(Commit::NoOperation, Phase::Empty))
            }
            _ => unreachable!(),
        }
    }

    /// Combinational `int_take` — drives the `int_entry` (zacw) capture
    /// at retire edges and the HALT-wake landing. The instruction-
    /// boundary input is implicit: this is only called from `retire_edge`
    /// and `halt_wake_edge`, which only run at retire / halt-wake CLK9↑s.
    fn int_take(&self) -> bool {
        self.interrupt_pending && self.ime.output() == InterruptMasterEnable::Enabled
    }

    /// Pure decode — returns the decoded Instruction with its Phase and
    /// retire-edge Commit. Does not mutate IME/dispatch state.
    /// `retire_edge` owns those mutations so `int_take`'s snapshot and
    /// the EI/DI int_entry-chain gate stay coherent.
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

    /// Retire edge — the dispatching CLK9↑ where `int_entry` (zacw),
    /// the register-file / IME write-back DFFs, and the sequencer's
    /// next-M-cycle BusAction all resolve on the same edge.
    ///
    /// On hardware zacw, the register-file write-back DFFs and the
    /// IME DFF share this capture edge; the data inputs to the
    /// write-back DFFs and the sequencer's BusAction selector are
    /// combinational functions of the freshly-captured `int_entry.q`.
    /// When `int_entry.q` resolves true the in-flight retire is
    /// suppressed and the next M-cycle is dispatch's M1; when false the
    /// retire's mutations land and the sequencer chains into a
    /// speculative fetch / execute.
    ///
    /// Sample `int_take` *before* `apply_commit` runs so RETI's IME
    /// write does not pollute its own dispatch-eligibility check —
    /// hardware reads the pre-edge IME view for `int_take` regardless
    /// of where the retiring instruction's IME write lands.
    ///
    /// Sequence:
    ///   1. Derive zaij/zkog gate from the Commit variant (EI/DI block).
    ///   2. Sample `int_take` (pre-`apply_commit` IME view).
    ///   3. Capture `int_entry` (zacw): `D = !gate AND int_take`.
    ///   4. Branch on `int_entry.q`:
    ///      - dispatch arm: drop halt, set
    ///        `CpuPhase::InterruptDispatch{step:0}`, tail-call
    ///        `mcycle_isr(0)` (which clears IME and emits the M1
    ///        Internal{pc} BusAction).
    ///      - fetch arm: `apply_commit`, HALT-bug check, jump-target
    ///        consume, pc sync, boundary, then chain `mcycle_fetch(0)` /
    ///        `mcycle_execute(0)` for the speculative next M-cycle.
    pub(super) fn retire_edge(&mut self, commit: Commit, next_phase: Phase) -> BusAction {
        let int_entry_gated =
            matches!(commit, Commit::EnableInterrupts | Commit::DisableInterrupts);

        // Sample int_take BEFORE apply_commit so RETI's own IME write
        // (in apply_commit) does not pollute the pre-edge IME view.
        let int_take = self.int_take();
        self.int_entry.write(!int_entry_gated && int_take);
        self.int_entry.tick();

        if self.int_entry.output() {
            // Dispatch arm — write-back is suppressed; the sequencer
            // hands `mcycle_isr(0)` the dispatching CLK9↑.
            self.halt_state = HaltState::Running;
            let pc = self.pc;
            let pc_hi = (pc >> 8) as u8;
            let pc_lo = (pc & 0xff) as u8;
            let sp = self.stack_pointer;
            self.phase = CpuPhase::InterruptDispatch {
                sp,
                pc_hi,
                pc_lo,
                step: 0,
            };
            self.exec_step = 0;
            self.pending_vector_resolve = false;
            self.boundary_flag = true;
            self.instruction_pc = pc;
            // Tail-call mcycle_isr(0): clears IME and emits Internal{pc}
            // as M-cycle Y+1's held BusAction (dispatch's M1).
            self.mcycle_isr(0)
                .expect("mcycle_isr step 0 must return Some")
        } else {
            // Fetch arm — apply the in-flight retire's write-back and
            // chain into the speculative next M-cycle.
            Self::apply_commit(self, commit);
            self.check_halt_bug();
            if let Some(target) = self.pending_jump_target.take() {
                self.bus_counter = target;
            }
            self.pc = self.bus_counter;
            self.boundary_flag = true;
            self.instruction_pc = self.bus_counter;

            match next_phase {
                Phase::Empty => {
                    self.phase = CpuPhase::Fetch;
                    self.exec_step = 0;
                    self.op_state = 0;
                    self.mcycle_fetch(0).expect("fetch step 0 must return Some")
                }
                phase => {
                    self.phase = CpuPhase::Execute { phase, step: 0 };
                    self.exec_step = 0;
                    self.mcycle_execute(0)
                        .expect("mcycle_execute must return Some")
                }
            }
        }
    }

    /// HALT-wake landing edge — the boundary CLK9↑ where `mcycle_halted`
    /// just emitted the speculative wake fetch (`BusAction::Read{pc}`)
    /// and set `pending_halt_wake_dispatch`. Reads `int_take`; on a
    /// dispatching wake, rewrites `phase` to `InterruptDispatch{step:1}`,
    /// replaces the in-progress M-cycle's `current_action` with
    /// dispatch's M1 (`Internal{pc}`), and clears IME.
    ///
    /// HALT release is gated by g42 (not zacw), so this edge does NOT
    /// capture `int_entry`. Step 1 is used (rather than step 0 + a
    /// tail-call to `mcycle_isr(0)`) because the wake M-cycle's
    /// BusAction has already been emitted by `mcycle_halted` and is
    /// being replaced in place; the next `next_mcycle` call advances
    /// directly into step 1's `InternalOamBug{sp}`.
    pub fn halt_wake_edge(&mut self) {
        if !self.pending_halt_wake_dispatch {
            return;
        }
        self.pending_halt_wake_dispatch = false;

        if self.int_take() {
            let pc = self.pc;
            let pc_hi = (pc >> 8) as u8;
            let pc_lo = (pc & 0xff) as u8;
            let sp = self.stack_pointer;
            self.phase = CpuPhase::InterruptDispatch {
                sp,
                pc_hi,
                pc_lo,
                step: 1,
            };
            self.exec_step = 0;
            self.pending_vector_resolve = false;
            self.boundary_flag = true;
            self.instruction_pc = pc;
            self.ime.write_immediate(InterruptMasterEnable::Disabled);
            self.current_action = Some(BusAction::Internal { address: pc });
        }
    }

    /// HALT bug: HALT decoded with IME=0 and a pending IF resumes
    /// immediately and fails to increment PC on the next opcode fetch.
    /// EI;HALT does not exercise this path — EI's IME-set commits before
    /// HALT decodes, so HALT sees IME=Enabled.
    fn check_halt_bug(&mut self) {
        if !matches!(self.halt_state, HaltState::Halted | HaltState::Halting)
            || !self.interrupt_pending
        {
            return;
        }
        if self.ime.output() == InterruptMasterEnable::Disabled {
            self.halt_state = HaltState::Running;
            self.halt_bug = true;
        }
    }

    /// The dot position that produced the last `DotAction`. The executor
    /// needs this to check timing signals (boga, bowa, mopa) for hardware
    /// tick ordering and OAM bug timing.
    pub fn dot_for_execute(&self) -> BusDot {
        self.last_dot
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
        if self.pending_vector_resolve {
            self.pending_vector_resolve = false;
            true
        } else {
            false
        }
    }

    /// Update `interrupt_pending` from the priority-encoded `IF & IE`.
    /// Combinational on hardware (not IME-gated — the IME gate sits in
    /// `int_take`). The vector itself is resolved later via
    /// `pending_vector_resolve` reading `interrupts.triggered()` at the
    /// ISR's M3→M4 push.
    pub fn update_interrupt_state(
        &mut self,
        triggered: Option<super::super::interrupts::Interrupt>,
    ) {
        self.interrupt_pending = triggered.is_some();
    }

    /// Clock g42 (yoii) on its CLK9 capture edge. g42 drives the HALT
    /// release chain (g42 → ykua → halt RS-latch reset).
    pub fn tick_g42(&mut self) {
        self.g42.write(self.interrupt_pending);
        self.g42.tick();
    }
}

#[derive(Clone, Copy)]
enum PhaseTag {
    Fetch,
    Halted,
    Execute,
    InterruptDispatch,
}
