use super::super::{MemoryMapped, interrupts::Interrupt};
use super::{
    Cpu, InterruptMasterEnable,
    instructions::bit_shift::{Carry, Direction},
    instructions::{Instruction, Interrupt as InterruptInstruction},
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

/// Post-fetch behavior for an instruction. Fetch M-cycles are handled by
/// `GameBoy::step()` (which ticks hardware per byte read). The processor
/// only emits the remaining post-fetch M-cycles.
#[derive(Debug)]
enum Phase {
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

    /// Pop: 2 stack reads + optional trailing internal.
    Pop { sp: u16, action: PopAction },

    /// Push: 1 internal + 2 writes.
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

// ── Processor ──────────────────────────────────────────────────

/// Lazy state machine that yields one `BusAction` per M-cycle.
pub struct Processor {
    /// The decoded instruction, preserved for debugger display.
    #[allow(dead_code)]
    pub instruction: Instruction,
    step: u8,
    phase: Phase,
    /// Scratch byte for multi-read phases (Pop, CondReturn) to store
    /// the first read value until the second read completes.
    scratch: u8,
}

impl Processor {
    /// Create a processor for a halted NOP (CPU is halted, ticks once).
    pub fn halted_nop(pc: u16) -> Self {
        Self {
            instruction: Instruction::NoOperation,
            step: 0,
            phase: Phase::HaltedNop { fetch_pc: pc },
            scratch: 0,
        }
    }

    /// Create a processor for hardware interrupt dispatch.
    pub fn interrupt(cpu: &mut Cpu, interrupt: Interrupt, mapped: &mut MemoryMapped) -> Self {
        cpu.interrupt_master_enable = InterruptMasterEnable::Disabled;
        mapped.interrupts.clear(interrupt);
        cpu.halted = false;

        let pc = cpu.program_counter;
        let pc_hi = (pc >> 8) as u8;
        let pc_lo = (pc & 0xff) as u8;
        cpu.stack_pointer = cpu.stack_pointer.wrapping_sub(2);
        cpu.program_counter = interrupt.vector();

        Self {
            instruction: Instruction::NoOperation,
            step: 0,
            phase: Phase::InterruptDispatch {
                sp: cpu.stack_pointer,
                pc_hi,
                pc_lo,
            },
            scratch: 0,
        }
    }

    /// Create a processor for a decoded instruction.
    pub fn new(instruction: Instruction, cpu: &mut Cpu) -> Self {
        let phase = match &instruction {
            Instruction::Interrupt(InterruptInstruction::Await) => {
                cpu.halted = true;
                Phase::Empty
            }
            Instruction::Stop => {
                cpu.halted = true;
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

        Self {
            instruction,
            step: 0,
            phase,
            scratch: 0,
        }
    }

    /// Advance one M-cycle. Returns `None` when instruction is complete.
    /// `read_value` is the byte read during the previous cycle's `BusAction::Read`.
    pub fn next(&mut self, read_value: u8, cpu: &mut Cpu) -> Option<BusAction> {
        let step = self.step;
        self.step += 1;

        match &self.phase {
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
                    0 => Some(BusAction::Internal),
                    1 => Some(BusAction::Write {
                        address: sp.wrapping_add(1),
                        value: *hi,
                    }),
                    2 => Some(BusAction::Write {
                        address: sp,
                        value: *lo,
                    }),
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
                    0 => Some(BusAction::Internal),
                    1 => Some(BusAction::Write {
                        address: sp.wrapping_add(1),
                        value: *hi,
                    }),
                    2 => Some(BusAction::Write {
                        address: sp,
                        value: *lo,
                    }),
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
                    0 => Some(BusAction::Internal),
                    1 => Some(BusAction::Internal),
                    2 => Some(BusAction::Write {
                        address: sp.wrapping_add(1),
                        value: *pc_hi,
                    }),
                    3 => Some(BusAction::Write {
                        address: sp,
                        value: *pc_lo,
                    }),
                    4 => Some(BusAction::Internal),
                    _ => None,
                }
            }
        }
    }
}
