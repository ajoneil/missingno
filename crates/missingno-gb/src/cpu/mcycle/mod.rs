use super::{
    Cpu, EiDelay, HaltState, InterruptLatch, InterruptMasterEnable,
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
    /// No bus activity (internal CPU work).
    Internal,
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
    pub fn as_u8(self) -> u8 { self.0 }

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
#[derive(Debug)]
enum AluOp {
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
            Some(BusAction::Internal) => DotAction::Idle,
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
            // Emit the fetch read
            Some(BusAction::Read {
                address: self.program_counter,
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
                self.instruction_pc = self.program_counter;
                return self.mcycle_halted(0);
            }

            // Check for interrupt dispatch
            if self.g42_interrupt_pending && self.interrupt_latch.take_ready().is_some() {
                // Interrupt detected — enter ISR dispatch.
                // PC stays at pre-fetch value (not incremented).
                self.interrupt_master_enable = InterruptMasterEnable::Disabled;
                self.ei_delay = None;

                let pc = self.program_counter;
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
                return self.mcycle_isr(0);
            }

            // Normal opcode consumption — increment PC (with halt_bug check)
            let opcode = read_value;
            if self.halt_bug {
                self.halt_bug = false;
            } else {
                self.program_counter = self.program_counter.wrapping_add(1);
            }

            let needed = operand_count(opcode);
            if needed == 0 {
                // No operands — decode immediately
                let bytes = [opcode, 0, 0];
                self.decode_and_transition(bytes, 1);
                self.exec_step = 0;
                self.mcycle_execute(0)
            } else {
                // Need operand bytes — enter Execute with Operands phase
                self.phase = CpuPhase::Execute {
                    phase: Phase::Operands {
                        pc: self.program_counter,
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
    /// Checks the interrupt pipeline. When an IME=1 interrupt is detected,
    /// emits a wakeup NOP Read[PC] and defers ISR dispatch to the next
    /// M-cycle via `halt_isr_dispatch_pending`. Each call is exactly one
    /// M-cycle.
    fn mcycle_halted(&mut self, _read_value: u8) -> Option<BusAction> {
        // ── Boundary housekeeping ──
        self.exec_step = 0;
        self.boundary_flag = true;
        self.instruction_pc = self.program_counter;

        // Consume g42_mid_mcycle: snapshot from dot 1 of the previous
        // M-cycle. If an interrupt fired at dot 1, the g42->g43->g49
        // cascade had enough time (3 CLK9 edges) to propagate.
        let g42_mid = self.g42_mid_mcycle;
        self.g42_mid_mcycle = false;

        // ── IME=1 wakeup: deferred ISR dispatch ──
        // If the wakeup NOP was emitted last M-cycle, now dispatch to ISR.
        if self.halt_isr_dispatch_pending {
            self.halt_isr_dispatch_pending = false;
            self.interrupt_master_enable = InterruptMasterEnable::Disabled;
            self.ei_delay = None;
            self.halt_state = HaltState::Running;

            let pc = self.program_counter;
            let pc_hi = (pc >> 8) as u8;
            let pc_lo = (pc & 0xff) as u8;
            let sp = self.stack_pointer;

            self.phase = CpuPhase::InterruptDispatch {
                sp,
                pc_hi,
                pc_lo,
                step: 0,
            };
            self.pending_vector_resolve = false;
            return self.mcycle_isr(0);
        }

        // ── IME=1 wakeup ──
        // Use combinational interrupt_pending (updated after PPU rise on
        // every dot) rather than the DFF-latched g42.  HALT has no
        // pipeline to drain, so the combinational state is correct.
        if self.interrupt_pending && self.interrupt_latch.take_ready().is_some() {
            if self.g42_was_pending || g42_mid {
                // g42 was already latched at the prior M-cycle boundary —
                // the g42→g43→g49 pipeline propagated during the idle
                // M-cycle. Skip the wakeup NOP and dispatch ISR directly.
                self.interrupt_master_enable = InterruptMasterEnable::Disabled;
                self.ei_delay = None;
                self.halt_state = HaltState::Running;

                let pc = self.program_counter;
                let pc_hi = (pc >> 8) as u8;
                let pc_lo = (pc & 0xff) as u8;
                let sp = self.stack_pointer;

                self.phase = CpuPhase::InterruptDispatch {
                    sp,
                    pc_hi,
                    pc_lo,
                    step: 0,
                };
                self.pending_vector_resolve = false;
                return self.mcycle_isr(0);
            } else {
                // g42 just latched this boundary — emit Read[PC] as the
                // wakeup NOP. A dummy fetch that is discarded. On hardware,
                // PC increments here and ISR M0 decrements it back; we skip
                // both for the same net effect.
                self.halt_isr_dispatch_pending = true;
                self.advance_ei_delay();
                return Some(BusAction::Read {
                    address: self.program_counter,
                });
            }
        }

        // ── IME=0 wakeup: chain directly to fetch ──
        // The wakeup NOP IS the first instruction's opcode fetch on
        // hardware — one M-cycle, not two. When the pending flag is
        // consumed, transition straight to Fetch phase.
        if self.interrupt_pending && self.halt_wakeup_pending {
            self.halt_wakeup_pending = false;
            self.halt_state = HaltState::Running;
            self.advance_ei_delay();
            self.phase = CpuPhase::Fetch;
            self.exec_step = 0;
            return self.mcycle_fetch(0);
        }

        // ── Still halted ──
        self.advance_ei_delay();
        Some(BusAction::Read {
            address: self.program_counter,
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
                self.program_counter = *pc;

                if *bytes_read >= *bytes_needed {
                    let b = *bytes;
                    let n = *bytes_read;
                    self.decode_and_transition(b, n);
                    if let CpuPhase::Execute { step, .. } = &mut self.phase {
                        *step = 0;
                    }
                    let action = self.mcycle_execute(0);
                    return (action, false);
                }

                (Some(BusAction::Read { address: *pc }), true)
            }

            Phase::Empty => (Some(self.enter_fetch()), false),

            Phase::ReadOp { address, action } => match current_step {
                0 => (Some(BusAction::Read { address: *address }), true),
                _ => {
                    Self::apply_read_action(self, action, read_value);
                    (Some(self.enter_fetch()), false)
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
                    _ => (Some(self.enter_fetch()), false),
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
                _ => (Some(self.enter_fetch()), false),
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
                    _ => (Some(self.enter_fetch()), false),
                }
            }

            Phase::InternalOp { count } => {
                if current_step < *count {
                    (Some(BusAction::Internal), true)
                } else {
                    (Some(self.enter_fetch()), false)
                }
            }

            Phase::InternalOamBug { address } => match current_step {
                0 => (Some(BusAction::InternalOamBug { address: *address }), true),
                _ => (Some(self.enter_fetch()), false),
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
                            (Some(BusAction::Internal), true)
                        } else {
                            (Some(self.enter_fetch()), false)
                        }
                    }
                    _ => (Some(self.enter_fetch()), false),
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
                    _ => (Some(self.enter_fetch()), false),
                }
            }

            Phase::CondJump { taken, target } => {
                if current_step == 0 && *taken {
                    // Internal M-cycle: the CPU loads the target address
                    // into PC. On hardware, PC updates here — not during
                    // decode when operands were read.
                    self.program_counter = *target;
                    (Some(BusAction::Internal), true)
                } else {
                    (Some(self.enter_fetch()), false)
                }
            }

            Phase::CondCall { taken, sp, hi, lo } => {
                if !*taken {
                    return (Some(self.enter_fetch()), false);
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
                    _ => (Some(self.enter_fetch()), false),
                }
            }

            Phase::CondReturn { taken, sp, action } => {
                let sp = *sp;
                let taken = *taken;
                match current_step {
                    0 => (Some(BusAction::Internal), true),
                    1 if !taken => (Some(self.enter_fetch()), false),
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
                        (Some(BusAction::Internal), true)
                    }
                    _ => (Some(self.enter_fetch()), false),
                }
            }
        }
    }

    /// ISR dispatch: 3 post-fetch M-cycles.
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
            // M0: IDU PC- — on hardware this undoes the wakeup NOP's PC
            // increment. The emulator skips both the increment and decrement
            // for the same net effect.
            0 => Some(BusAction::Internal),
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
                Some(self.enter_fetch())
            }
            _ => unreachable!(),
        }
    }

    /// Transition from fetch to execution phase after all bytes are read.
    fn decode_and_transition(&mut self, bytes: [u8; 3], bytes_read: u8) {
        let mut iter = bytes[..bytes_read as usize].iter().copied();
        let instruction = Instruction::decode(&mut iter).unwrap();

        let phase = match &instruction {
            Instruction::Interrupt(InterruptInstruction::Await) => {
                self.halt_state = HaltState::Halting;
                Phase::Empty
            }
            Instruction::Stop => {
                self.halt_state = HaltState::Halting;
                Phase::Empty
            }
            Instruction::Invalid(op) => panic!("Invalid instruction {:02x}", op),

            Instruction::NoOperation => Phase::Empty,
            Instruction::DecimalAdjustAccumulator => {
                Self::apply_daa(self);
                Phase::Empty
            }
            Instruction::CarryFlag(cf) => {
                Self::apply_carry_flag(self, cf);
                Phase::Empty
            }
            Instruction::Interrupt(instr) => {
                Self::apply_interrupt_instruction(self, instr);
                Phase::Empty
            }

            Instruction::Load(load) => Self::build_load(self, load),
            Instruction::Arithmetic(arith) => Self::build_arithmetic(self, arith),
            Instruction::Bitwise(bw) => Self::build_bitwise(self, bw),
            Instruction::BitShift(bs) => Self::build_bit_shift(self, bs),
            Instruction::BitFlag(bf) => Self::build_bit_flag(self, bf),
            Instruction::Jump(j) => Self::build_jump(self, j),
            Instruction::Stack(s) => Self::build_stack(self, s),
        };

        self.instruction = instruction;
        self.phase = CpuPhase::Execute { phase, step: 0 };
    }

    /// Transition to the Fetch phase, run instruction-boundary side
    /// effects (HALT bug check, EI delay advance), signal the boundary,
    /// and chain into the first fetch M-cycle.
    fn enter_fetch(&mut self) -> BusAction {
        self.phase = CpuPhase::Fetch;
        self.exec_step = 0;
        self.op_state = 0;

        // Run HALT bug check and EI delay advance at the instruction
        // boundary, INSIDE the CPU, so the timing is exact regardless
        // of when the executor detects the boundary.
        self.check_halt_bug();
        self.advance_ei_delay();

        self.boundary_flag = true;
        self.instruction_pc = self.program_counter;

        // Chain into the first fetch M-cycle. If check_halt_bug
        // transitioned to Halting/Halted, mcycle_fetch will handle it.
        self.mcycle_fetch(0).expect("fetch step 0 must return Some")
    }

    /// HALT bug: if HALT was just executed with IME=0 and an interrupt
    /// is already pending, the CPU doesn't truly halt. It resumes
    /// immediately but fails to increment PC on the next opcode fetch.
    fn check_halt_bug(&mut self) {
        if !matches!(self.halt_state, HaltState::Halted | HaltState::Halting)
            || !self.interrupt_pending
        {
            return;
        }
        if self.ei_delay == Some(EiDelay::Fired) {
            // EI immediately before HALT: on real hardware HALT saw
            // IME=0 (the DFF pipeline hadn't propagated yet). The halt
            // bug triggers — PC is not incremented. But EI's IME
            // promotion still takes effect, so the interrupt dispatches.
            self.interrupt_master_enable = InterruptMasterEnable::Enabled;
            self.program_counter = self.program_counter.wrapping_sub(1);
            self.halt_state = HaltState::Running;
            self.ei_delay = None;
        } else if self.interrupt_master_enable == InterruptMasterEnable::Disabled {
            self.halt_state = HaltState::Running;
            self.halt_bug = true;
        }
    }

    /// Advance the EI delay pipeline one stage per instruction
    /// completion, modeling the DFF cascade from EI's decode signal
    /// to the IME flip-flop.
    fn advance_ei_delay(&mut self) {
        self.ei_delay = match self.ei_delay {
            Some(EiDelay::Pending) => Some(EiDelay::Fired),
            Some(EiDelay::Fired) => {
                self.interrupt_master_enable = InterruptMasterEnable::Enabled;
                None
            }
            None => None,
        };
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

    /// Update the interrupt latch based on externally-provided trigger state.
    /// Called every dot by the executor after both phases have ticked.
    pub fn update_interrupt_state(
        &mut self,
        triggered: Option<super::super::interrupts::Interrupt>,
    ) {
        self.interrupt_pending = triggered.is_some();

        self.interrupt_latch = match self.interrupt_master_enable {
            InterruptMasterEnable::Enabled => match triggered {
                Some(interrupt) => InterruptLatch::Ready(interrupt),
                None => InterruptLatch::Empty,
            },
            InterruptMasterEnable::Disabled => InterruptLatch::Empty,
        };

        // IME=0 halt wakeup: set the pending flag instead of immediately
        // transitioning to Running. The current idle M-cycle completes as
        // idle; mcycle_halted consumes the flag at the next M-cycle boundary.
        if self.halt_state == HaltState::Halted
            && self.interrupt_master_enable == InterruptMasterEnable::Disabled
            && triggered.is_some()
        {
            self.halt_wakeup_pending = true;
        }
    }
}

#[derive(Clone, Copy)]
enum PhaseTag {
    Fetch,
    Halted,
    Execute,
    InterruptDispatch,
}
