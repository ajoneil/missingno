use super::{
    Cpu, HaltState, InterruptMasterEnable,
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
pub enum BusAction {
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
    /// Used for: DriveBus timing -- PPU register DFFs see the write
    /// data during this window.
    pub fn bude(self) -> bool {
        self.0 >= 2
    }

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
    /// a named signal doesn't exist (e.g., TrailingFetch counter).
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
/// - **Write**:    `[Idle, Idle, DriveBus, Write]`
/// - **Internal**: `[Idle, Idle, Idle, Idle]`
/// - **OamBug**:   `[InternalOamBug, Idle, Idle, Idle]`
#[derive(Debug)]
pub enum DotAction {
    /// No bus transfer this dot.
    Idle,
    /// CPU is reading from this address. The executor must provide
    /// the value before the next M-cycle begins.
    Read { address: u16 },
    /// CPU is writing this value to this address. The write latches
    /// at this dot (G→H boundary, end of M-cycle).
    Write { address: u16, value: u8 },
    /// The CPU write pulse is active — the data bus is being driven with
    /// the write value. PPU register DFF cells latch from the bus at this
    /// dot (phases E,F,G of the write M-cycle). The bus write completes
    /// at dot 3 via `Write`.
    DriveBus { address: u16, value: u8 },
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

/// The behavior of the current instruction, expressed as a sequence of
/// M-cycles. The `Processor` walks through the phase yielding one
/// `BusAction` per M-cycle via `next_mcycle()`.
#[derive(Debug)]
enum Phase {
    /// Fetch opcode and operand bytes, decode, then transition to the
    /// instruction's execution phase. Each byte is one Read M-cycle.
    Fetch {
        pc: u16,
        /// Bytes read so far (opcode + operands). Index 0 = opcode.
        bytes: [u8; 3],
        /// How many bytes have been read.
        bytes_read: u8,
        /// Total bytes needed (0 = unknown until opcode is read).
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
    CondJump { taken: bool },

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

    /// Interrupt dispatch: 5 M-cycles (no decode).
    InterruptDispatch { sp: u16, pc_hi: u8, pc_lo: u8 },

    /// Halted NOP: 1 fetch Read (no decode happens when halted).
    HaltedNop { fetch_pc: u16 },

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

// ── Processor ──────────────────────────────────────────────────

/// State machine that yields one `DotAction` per dot (T-cycle).
///
/// Each instruction is a sequence of M-cycles, each expanded into 4 dots.
/// The processor covers the entire instruction lifecycle: fetch (reading
/// opcode + operands), decode, and execution (post-fetch M-cycles).
pub struct Processor {
    /// The decoded instruction, preserved for debugger display.
    #[allow(dead_code)]
    pub instruction: Instruction,
    step: u8,
    phase: Phase,
    /// Scratch byte for multi-read phases (Pop, CondReturn) to store
    /// the first read value until the second read completes.
    scratch: u8,
    /// Dot position within the current M-cycle (0–3).
    dot: BusDot,
    /// The BusAction for the current M-cycle, fetched at dot 0.
    /// `None` means the instruction is complete.
    current_action: Option<BusAction>,
    /// Whether we have started dot iteration (have a pending M-cycle).
    mcycle_active: bool,
    /// IE push bug: set between the high-byte and low-byte push of
    /// interrupt dispatch. The high-byte push may have written to the
    /// IE register at 0xFFFF, so the caller must sample IF & IE at
    /// this point to determine the jump vector — before the low-byte
    /// push, which must not affect the vector.
    pending_vector_resolve: bool,
}

impl Processor {
    /// Create a processor for a halted CPU. The CPU is idling — it
    /// drives PC onto the address bus (one Read M-cycle) but discards
    /// the result. Hardware ticks during this M-cycle may wake the CPU.
    pub fn begin(cpu: &mut Cpu) -> Self {
        debug_assert!(
            matches!(cpu.halt_state, HaltState::Halted | HaltState::Halting),
            "begin() called with CPU not halted — prefetched_opcode should be set"
        );
        Self::halted_nop(cpu.program_counter)
    }

    /// Create a processor for a halted NOP (CPU is halted, ticks once).
    fn halted_nop(pc: u16) -> Self {
        Self {
            instruction: Instruction::NoOperation,
            step: 0,
            phase: Phase::HaltedNop { fetch_pc: pc },
            scratch: 0,
            dot: BusDot::ZERO,
            current_action: None,
            mcycle_active: false,
            pending_vector_resolve: false,
        }
    }

    /// Create a processor that skips the opcode read M-cycle.
    /// The opcode has already been fetched; the Processor starts at the
    /// point where it would consume `read_value` as the opcode byte.
    pub fn fetch_with_opcode(cpu: &mut Cpu, opcode: u8) -> Self {
        let mut proc = Self {
            instruction: Instruction::NoOperation,
            step: 1,
            phase: Phase::Fetch {
                pc: cpu.program_counter,
                bytes: [0; 3],
                bytes_read: 0,
                bytes_needed: 0,
            },
            scratch: 0,
            dot: BusDot::ZERO,
            current_action: None,
            mcycle_active: false,
            pending_vector_resolve: false,
        };
        proc.current_action = proc.next_mcycle(opcode, cpu);
        if proc.current_action.is_some() {
            proc.mcycle_active = true;
        }
        proc
    }

    /// Create a processor for hardware interrupt dispatch.
    ///
    /// The jump vector is not resolved here — it is deferred until after
    /// the high-byte push via `take_pending_vector_resolve()`, because
    /// the push may land on the IE register at 0xFFFF and alter which
    /// interrupt is triggered (IE push bug).
    pub fn interrupt(cpu: &mut Cpu) -> Self {
        cpu.interrupt_master_enable = InterruptMasterEnable::Disabled;
        cpu.ei_delay = None;
        cpu.halt_state = HaltState::Running;

        let pc = cpu.program_counter;
        let pc_hi = (pc >> 8) as u8;
        let pc_lo = (pc & 0xff) as u8;
        let sp = cpu.stack_pointer;

        Self {
            instruction: Instruction::NoOperation,
            step: 0,
            phase: Phase::InterruptDispatch { sp, pc_hi, pc_lo },
            scratch: 0,
            dot: BusDot::ZERO,
            current_action: None,
            mcycle_active: false,
            pending_vector_resolve: false,
        }
    }

    /// Transition from fetch to execution phase after all bytes are read.
    fn decode_and_transition(&mut self, cpu: &mut Cpu, bytes: [u8; 3], bytes_read: u8) {
        let mut iter = bytes[..bytes_read as usize].iter().copied();
        let instruction = Instruction::decode(&mut iter).unwrap();

        let phase = match &instruction {
            Instruction::Interrupt(InterruptInstruction::Await) => {
                cpu.halt_state = HaltState::Halting;
                Phase::Empty
            }
            Instruction::Stop => {
                cpu.halt_state = HaltState::Halting;
                Phase::Empty
            }
            Instruction::Invalid(op) => panic!("Invalid instruction {:02x}", op),

            Instruction::NoOperation => Phase::Empty,
            Instruction::DecimalAdjustAccumulator => {
                Self::apply_daa(cpu);
                Phase::Empty
            }
            Instruction::CarryFlag(cf) => {
                Self::apply_carry_flag(cpu, cf);
                Phase::Empty
            }
            Instruction::Interrupt(instr) => {
                Self::apply_interrupt_instruction(cpu, instr);
                Phase::Empty
            }

            Instruction::Load(load) => Self::build_load(cpu, load),
            Instruction::Arithmetic(arith) => Self::build_arithmetic(cpu, arith),
            Instruction::Bitwise(bw) => Self::build_bitwise(cpu, bw),
            Instruction::BitShift(bs) => Self::build_bit_shift(cpu, bs),
            Instruction::BitFlag(bf) => Self::build_bit_flag(cpu, bf),
            Instruction::Jump(j) => Self::build_jump(cpu, j),
            Instruction::Stack(s) => Self::build_stack(cpu, s),
        };

        self.instruction = instruction;
        self.phase = phase;
        self.step = 0;
    }

    /// Advance one M-cycle. Returns `None` when instruction is complete.
    /// `read_value` is the byte read during the previous cycle's `BusAction::Read`.
    fn next_mcycle(&mut self, read_value: u8, cpu: &mut Cpu) -> Option<BusAction> {
        let step = self.step;
        self.step += 1;

        match &mut self.phase {
            Phase::Fetch {
                pc,
                bytes,
                bytes_read,
                bytes_needed,
            } => {
                if step == 0 {
                    // First M-cycle: read opcode
                    Some(BusAction::Read { address: *pc })
                } else if *bytes_read == 0 {
                    // Opcode just read — store it and determine operand count
                    bytes[0] = read_value;
                    *bytes_read = 1;
                    if cpu.halt_bug {
                        cpu.halt_bug = false;
                    } else {
                        *pc += 1;
                    }
                    cpu.program_counter = *pc;
                    *bytes_needed = 1 + operand_count(bytes[0]);
                    if *bytes_read >= *bytes_needed {
                        // No operands — decode and transition
                        let b = *bytes;
                        let n = *bytes_read;
                        self.decode_and_transition(cpu, b, n);
                        // Check HALT bug after decode
                        self.check_halt_bug_after_decode(cpu);
                        // Return first M-cycle of execution phase (or None if Empty)
                        self.step = 0;
                        self.next_mcycle(0, cpu)
                    } else {
                        // Read next operand byte
                        Some(BusAction::Read { address: *pc })
                    }
                } else {
                    // Operand byte just read
                    bytes[*bytes_read as usize] = read_value;
                    *bytes_read += 1;
                    *pc += 1;
                    cpu.program_counter = *pc;
                    if *bytes_read >= *bytes_needed {
                        // All bytes read — decode and transition
                        let b = *bytes;
                        let n = *bytes_read;
                        self.decode_and_transition(cpu, b, n);
                        self.check_halt_bug_after_decode(cpu);
                        self.step = 0;
                        self.next_mcycle(0, cpu)
                    } else {
                        // Read next operand byte
                        Some(BusAction::Read { address: *pc })
                    }
                }
            }

            Phase::Empty => None,

            Phase::HaltedNop { fetch_pc } => match step {
                0 => Some(BusAction::Read { address: *fetch_pc }),
                _ => None,
            },

            Phase::ReadOp { address, action } => match step {
                0 => Some(BusAction::Read { address: *address }),
                1 => {
                    Self::apply_read_action(cpu, action, read_value);
                    None
                }
                _ => None,
            },

            Phase::ReadModifyWrite { address, op } => {
                let address = *address;
                match step {
                    0 => Some(BusAction::Read { address }),
                    1 => {
                        let result = Self::apply_rmw(cpu, op, read_value);
                        Some(BusAction::Write {
                            address,
                            value: result,
                        })
                    }
                    _ => None,
                }
            }

            Phase::WriteOp {
                address,
                value,
                hl_post,
            } => match step {
                0 => {
                    if *hl_post != 0 {
                        let hl = cpu.get_register16(Register16::Hl);
                        cpu.set_register16(Register16::Hl, hl.wrapping_add(*hl_post as u16));
                    }
                    Some(BusAction::Write {
                        address: *address,
                        value: *value,
                    })
                }
                _ => None,
            },

            Phase::Write16 { address, lo, hi } => {
                let address = *address;
                match step {
                    0 => Some(BusAction::Write {
                        address,
                        value: *lo,
                    }),
                    1 => Some(BusAction::Write {
                        address: address.wrapping_add(1),
                        value: *hi,
                    }),
                    _ => None,
                }
            }

            Phase::InternalOp { count } => {
                if step < *count {
                    Some(BusAction::Internal)
                } else {
                    None
                }
            }

            Phase::InternalOamBug { address } => match step {
                0 => Some(BusAction::InternalOamBug { address: *address }),
                _ => None,
            },

            Phase::Pop { sp, action } => {
                let sp = *sp;
                match step {
                    0 => Some(BusAction::Read { address: sp }),
                    1 => {
                        self.scratch = read_value;
                        Some(BusAction::Read {
                            address: sp.wrapping_add(1),
                        })
                    }
                    2 => {
                        Self::apply_pop(cpu, action, self.scratch, read_value, sp);
                        let has_trailing =
                            matches!(action, PopAction::SetPc | PopAction::SetPcEnableInterrupts);
                        if has_trailing {
                            Some(BusAction::Internal)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }

            Phase::Push { sp, hi, lo } => {
                let sp = *sp;
                match step {
                    0 => Some(BusAction::InternalOamBug { address: sp }),
                    1 => {
                        // First decrement: SP-1, write high byte
                        let addr = sp.wrapping_sub(1);
                        cpu.stack_pointer = addr;
                        Some(BusAction::Write {
                            address: addr,
                            value: *hi,
                        })
                    }
                    2 => {
                        // Second decrement: SP-2, write low byte
                        let addr = sp.wrapping_sub(2);
                        cpu.stack_pointer = addr;
                        Some(BusAction::Write {
                            address: addr,
                            value: *lo,
                        })
                    }
                    _ => None,
                }
            }

            Phase::CondJump { taken } => match step {
                0 if *taken => Some(BusAction::Internal),
                _ => None,
            },

            Phase::CondCall { taken, sp, hi, lo } => {
                if !*taken {
                    return None;
                }
                let sp = *sp;
                match step {
                    0 => Some(BusAction::InternalOamBug { address: sp }),
                    1 => {
                        let addr = sp.wrapping_sub(1);
                        cpu.stack_pointer = addr;
                        Some(BusAction::Write {
                            address: addr,
                            value: *hi,
                        })
                    }
                    2 => {
                        let addr = sp.wrapping_sub(2);
                        cpu.stack_pointer = addr;
                        Some(BusAction::Write {
                            address: addr,
                            value: *lo,
                        })
                    }
                    _ => None,
                }
            }

            Phase::CondReturn { taken, sp, action } => {
                let sp = *sp;
                let taken = *taken;
                match step {
                    0 => Some(BusAction::Internal),
                    1 if !taken => None,
                    1 => Some(BusAction::Read { address: sp }),
                    2 => {
                        self.scratch = read_value;
                        Some(BusAction::Read {
                            address: sp.wrapping_add(1),
                        })
                    }
                    3 => {
                        Self::apply_pop(cpu, action, self.scratch, read_value, sp);
                        Some(BusAction::Internal)
                    }
                    _ => None,
                }
            }

            Phase::InterruptDispatch { sp, pc_hi, pc_lo } => {
                let sp = *sp;
                match step {
                    0 => {
                        let pc = (*pc_hi as u16) << 8 | *pc_lo as u16;
                        Some(BusAction::InternalOamBug { address: pc })
                    }
                    1 => Some(BusAction::InternalOamBug { address: sp }),
                    2 => {
                        let addr = sp.wrapping_sub(1);
                        cpu.stack_pointer = addr;
                        Some(BusAction::Write {
                            address: addr,
                            value: *pc_hi,
                        })
                    }
                    3 => {
                        // IE push bug: the vector must be resolved after
                        // the high-byte push (step 2) but before this
                        // low-byte push. The high-byte write may have
                        // landed on IE at 0xFFFF.
                        self.pending_vector_resolve = true;
                        let addr = sp.wrapping_sub(2);
                        cpu.stack_pointer = addr;
                        Some(BusAction::Write {
                            address: addr,
                            value: *pc_lo,
                        })
                    }
                    _ => None,
                }
            }
        }
    }

    /// HALT bug: if HALT was just executed with IME=0 and an interrupt
    /// is already pending, the CPU doesn't truly halt. It resumes
    /// immediately but fails to increment PC on the next opcode fetch.
    ///
    /// Called after decode when the instruction set `cpu.halted = true`.
    fn check_halt_bug_after_decode(&self, _cpu: &mut Cpu) {
        // This is checked by the execute loop which has access to the
        // interrupt registers. The Processor just sets halted; the
        // execute loop detects halted + pending interrupt and sets
        // halt_bug or rewinds PC as appropriate.
    }

    /// Advance one dot (T-cycle). Returns `None` when the instruction
    /// is complete.
    ///
    /// Each M-cycle is expanded into 4 dots with bus operations at the
    /// hardware-correct position:
    /// - **Read**:     `[Idle, Idle, Idle, Read]`
    /// - **Write**:    `[Idle, Idle, DriveBus, Write]`
    /// - **Internal**: `[Idle, Idle, Idle, Idle]`
    /// - **OamBug**:   `[InternalOamBug, Idle, Idle, Idle]`
    ///
    /// `read_value` is consumed at the start of a new M-cycle (dot 0)
    /// when the previous M-cycle was a Read.
    pub fn next_dot(&mut self, read_value: u8, cpu: &mut Cpu) -> Option<DotAction> {
        // At the start of a new M-cycle, fetch the next BusAction
        if !self.mcycle_active {
            self.current_action = self.next_mcycle(read_value, cpu);
            if self.current_action.is_none() {
                return None;
            }
            self.dot = BusDot::ZERO;
            self.mcycle_active = true;
        }

        let dot = self.dot;
        self.dot = if dot.boga() {
            BusDot::ZERO
        } else {
            dot.advance()
        };

        let result = match &self.current_action {
            // Read: data capture at BOGA (M-cycle boundary, dot 3)
            Some(BusAction::Read { address }) => {
                if dot.boga() {
                    DotAction::Read { address: *address }
                } else {
                    DotAction::Idle
                }
            }
            // Write: drive data at BUDE, latch at AFAS falling
            Some(BusAction::Write { address, value }) => {
                if dot.bude() && !dot.afas_falling() {
                    // Dot 2: BUDE rises, write data appears on bus
                    DotAction::DriveBus {
                        address: *address,
                        value: *value,
                    }
                } else if dot.afas_falling() {
                    // Dot 3: AFAS falls at G->H, register DFFs latch
                    DotAction::Write {
                        address: *address,
                        value: *value,
                    }
                } else {
                    DotAction::Idle
                }
            }
            // OAM bug: IDU address on bus at BOWA
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

        Some(result)
    }

    /// If the current M-cycle has an IDU address on the bus (from an
    /// `InternalOamBug` action), returns that address for OAM bug write
    /// corruption.
    pub fn oam_bug_address(&self) -> Option<u16> {
        match &self.current_action {
            Some(BusAction::InternalOamBug { address }) => Some(*address),
            _ => None,
        }
    }

    /// IE push bug: consume the pending vector resolution request.
    ///
    /// During interrupt dispatch, the high-byte push (to `[SP-1]`) may
    /// land on the IE register at 0xFFFF. The hardware samples IF & IE
    /// after this write but before the low-byte push to determine the
    /// jump vector. Returns `true` exactly once, at the M-cycle
    /// boundary between the two push writes.
    pub fn take_pending_vector_resolve(&mut self) -> bool {
        if self.pending_vector_resolve {
            self.pending_vector_resolve = false;
            true
        } else {
            false
        }
    }
}
