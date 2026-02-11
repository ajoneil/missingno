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

/// Post-fetch behavior for an instruction. Each variant carries all data
/// needed so that `next()` only needs `&mut Cpu` for read-dependent mutations.
#[derive(Debug)]
enum Phase {
    /// Fetch-only: emit fetch Reads, no post-fetch M-cycles.
    FetchOnly { fetches: u8, fetch_pc: u16 },

    /// One memory read, then a CPU action.
    ReadOp {
        fetches: u8,
        fetch_pc: u16,
        address: u16,
        action: ReadAction,
    },

    /// Read-modify-write on a memory address.
    ReadModifyWrite {
        fetches: u8,
        fetch_pc: u16,
        address: u16,
        op: RmwOp,
    },

    /// One memory write.
    WriteOp {
        fetches: u8,
        fetch_pc: u16,
        address: u16,
        value: u8,
        hl_post: i16,
    },

    /// Two memory writes (LD [a16],SP).
    Write16 {
        fetches: u8,
        fetch_pc: u16,
        address: u16,
        lo: u8,
        hi: u8,
    },

    /// N internal cycles, no bus activity.
    InternalOp {
        fetches: u8,
        fetch_pc: u16,
        count: u8,
    },

    /// Pop: 2 stack reads + optional trailing internal.
    Pop {
        fetches: u8,
        fetch_pc: u16,
        sp: u16,
        action: PopAction,
    },

    /// Push: 1 internal + 2 writes.
    Push {
        fetches: u8,
        fetch_pc: u16,
        sp: u16,
        hi: u8,
        lo: u8,
    },

    /// Conditional jump: fetches + 0 or 1 internal.
    CondJump {
        fetches: u8,
        fetch_pc: u16,
        taken: bool,
    },

    /// Conditional call: fetches + (if taken: internal + 2 writes).
    CondCall {
        fetches: u8,
        fetch_pc: u16,
        taken: bool,
        sp: u16,
        hi: u8,
        lo: u8,
    },

    /// Conditional return: fetch + internal + (if taken: 2 reads + internal).
    CondReturn {
        fetches: u8,
        fetch_pc: u16,
        taken: bool,
        sp: u16,
        action: PopAction,
    },

    /// Interrupt dispatch: 5 M-cycles, no fetches.
    InterruptDispatch { sp: u16, pc_hi: u8, pc_lo: u8 },

    /// Halted NOP: 1 fetch Read, no action.
    HaltedNop { fetch_pc: u16 },

    /// HALT/STOP: 0 M-cycles.
    Empty,
}

// ── InstructionStepper ──────────────────────────────────────────────────

/// Lazy state machine that yields one `BusAction` per M-cycle.
pub struct InstructionStepper {
    /// The decoded instruction, preserved for debugger display.
    #[allow(dead_code)]
    pub instruction: Instruction,
    step: u8,
    phase: Phase,
    /// Scratch byte for multi-read phases (Pop, CondReturn) to store
    /// the first read value until the second read completes.
    scratch: u8,
}

impl InstructionStepper {
    /// Create a stepper for a halted NOP (CPU is halted, ticks once).
    pub fn halted_nop(pc: u16) -> Self {
        Self {
            instruction: Instruction::NoOperation,
            step: 0,
            phase: Phase::HaltedNop { fetch_pc: pc },
            scratch: 0,
        }
    }

    /// Create a stepper for hardware interrupt dispatch.
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

    /// Create a stepper for a decoded instruction.
    pub fn new(instruction: Instruction, cpu: &mut Cpu) -> Self {
        let fetch_count = instruction.fetch_byte_count() as u8;
        let fetch_pc = cpu.program_counter.wrapping_sub(fetch_count as u16);

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

            Instruction::NoOperation => Phase::FetchOnly {
                fetches: fetch_count,
                fetch_pc,
            },
            Instruction::DecimalAdjustAccumulator => {
                Self::apply_daa(cpu);
                Phase::FetchOnly {
                    fetches: fetch_count,
                    fetch_pc,
                }
            }
            Instruction::CarryFlag(cf) => {
                Self::apply_carry_flag(cpu, cf);
                Phase::FetchOnly {
                    fetches: fetch_count,
                    fetch_pc,
                }
            }
            Instruction::Interrupt(instr) => {
                Self::apply_interrupt_instruction(cpu, instr);
                Phase::FetchOnly {
                    fetches: fetch_count,
                    fetch_pc,
                }
            }

            Instruction::Load(load) => Self::build_load(cpu, load, fetch_count, fetch_pc),
            Instruction::Arithmetic(arith) => {
                Self::build_arithmetic(cpu, arith, fetch_count, fetch_pc)
            }
            Instruction::Bitwise(bw) => Self::build_bitwise(cpu, bw, fetch_count, fetch_pc),
            Instruction::BitShift(bs) => Self::build_bit_shift(cpu, bs, fetch_count, fetch_pc),
            Instruction::BitFlag(bf) => Self::build_bit_flag(cpu, bf, fetch_count, fetch_pc),
            Instruction::Jump(j) => Self::build_jump(cpu, j, fetch_count, fetch_pc),
            Instruction::Stack(s) => Self::build_stack(cpu, s, fetch_count, fetch_pc),
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

            Phase::FetchOnly { fetches, fetch_pc } => {
                if step < *fetches {
                    Some(BusAction::Read {
                        address: fetch_pc.wrapping_add(step as u16),
                    })
                } else {
                    None
                }
            }

            Phase::ReadOp {
                fetches,
                fetch_pc,
                address,
                action,
            } => {
                if step < *fetches {
                    Some(BusAction::Read {
                        address: fetch_pc.wrapping_add(step as u16),
                    })
                } else {
                    let post_fetch = step - *fetches;
                    match post_fetch {
                        0 => Some(BusAction::Read { address: *address }),
                        1 => {
                            Self::apply_read_action(cpu, action, read_value);
                            None
                        }
                        _ => None,
                    }
                }
            }

            Phase::ReadModifyWrite {
                fetches,
                fetch_pc,
                address,
                op,
            } => {
                let address = *address;
                if step < *fetches {
                    Some(BusAction::Read {
                        address: fetch_pc.wrapping_add(step as u16),
                    })
                } else {
                    let post_fetch = step - *fetches;
                    match post_fetch {
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
            }

            Phase::WriteOp {
                fetches,
                fetch_pc,
                address,
                value,
                hl_post,
            } => {
                let address = *address;
                let value = *value;
                let hl_post = *hl_post;
                if step < *fetches {
                    Some(BusAction::Read {
                        address: fetch_pc.wrapping_add(step as u16),
                    })
                } else {
                    let post_fetch = step - *fetches;
                    match post_fetch {
                        0 => {
                            if hl_post != 0 {
                                let hl = cpu.get_register16(Register16::Hl);
                                cpu.set_register16(Register16::Hl, hl.wrapping_add(hl_post as u16));
                            }
                            Some(BusAction::Write { address, value })
                        }
                        _ => None,
                    }
                }
            }

            Phase::Write16 {
                fetches,
                fetch_pc,
                address,
                lo,
                hi,
            } => {
                let address = *address;
                let lo = *lo;
                let hi = *hi;
                if step < *fetches {
                    Some(BusAction::Read {
                        address: fetch_pc.wrapping_add(step as u16),
                    })
                } else {
                    let post_fetch = step - *fetches;
                    match post_fetch {
                        0 => Some(BusAction::Write { address, value: lo }),
                        1 => Some(BusAction::Write {
                            address: address.wrapping_add(1),
                            value: hi,
                        }),
                        _ => None,
                    }
                }
            }

            Phase::InternalOp {
                fetches,
                fetch_pc,
                count,
            } => {
                let count = *count;
                if step < *fetches {
                    Some(BusAction::Read {
                        address: fetch_pc.wrapping_add(step as u16),
                    })
                } else {
                    let post_fetch = step - *fetches;
                    if post_fetch < count {
                        Some(BusAction::Internal)
                    } else {
                        None
                    }
                }
            }

            // Pop: fetches, Read [SP], Read [SP+1], optional trailing Internal
            // POP rr: fetch + read_lo + read_hi (3 M-cycles)
            // RET/RETI: fetch + read_lo + read_hi + internal (4 M-cycles)
            Phase::Pop {
                fetches,
                fetch_pc,
                sp,
                action,
            } => {
                let sp = *sp;
                if step < *fetches {
                    Some(BusAction::Read {
                        address: fetch_pc.wrapping_add(step as u16),
                    })
                } else {
                    let post_fetch = step - *fetches;
                    match post_fetch {
                        0 => Some(BusAction::Read { address: sp }),
                        1 => {
                            self.scratch = read_value;
                            Some(BusAction::Read {
                                address: sp.wrapping_add(1),
                            })
                        }
                        2 => {
                            Self::apply_pop(cpu, action, self.scratch, read_value, sp);
                            let has_trailing = matches!(
                                action,
                                PopAction::SetPc | PopAction::SetPcEnableInterrupts
                            );
                            if has_trailing {
                                Some(BusAction::Internal)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                }
            }

            // Push: fetches + Internal + Write [SP+1] (hi) + Write [SP] (lo)
            Phase::Push {
                fetches,
                fetch_pc,
                sp,
                hi,
                lo,
            } => {
                let sp = *sp;
                let hi = *hi;
                let lo = *lo;
                if step < *fetches {
                    Some(BusAction::Read {
                        address: fetch_pc.wrapping_add(step as u16),
                    })
                } else {
                    let post_fetch = step - *fetches;
                    match post_fetch {
                        0 => Some(BusAction::Internal),
                        1 => Some(BusAction::Write {
                            address: sp.wrapping_add(1),
                            value: hi,
                        }),
                        2 => Some(BusAction::Write {
                            address: sp,
                            value: lo,
                        }),
                        _ => None,
                    }
                }
            }

            // Conditional jump: fetches + (if taken: 1 internal)
            Phase::CondJump {
                fetches,
                fetch_pc,
                taken,
            } => {
                let taken = *taken;
                if step < *fetches {
                    Some(BusAction::Read {
                        address: fetch_pc.wrapping_add(step as u16),
                    })
                } else {
                    let post_fetch = step - *fetches;
                    match post_fetch {
                        0 if taken => Some(BusAction::Internal),
                        _ => None,
                    }
                }
            }

            // Conditional call: fetches + (if taken: internal + write_hi + write_lo)
            Phase::CondCall {
                fetches,
                fetch_pc,
                taken,
                sp,
                hi,
                lo,
            } => {
                let sp = *sp;
                let hi = *hi;
                let lo = *lo;
                let taken = *taken;
                if step < *fetches {
                    Some(BusAction::Read {
                        address: fetch_pc.wrapping_add(step as u16),
                    })
                } else if !taken {
                    None
                } else {
                    let post_fetch = step - *fetches;
                    match post_fetch {
                        0 => Some(BusAction::Internal),
                        1 => Some(BusAction::Write {
                            address: sp.wrapping_add(1),
                            value: hi,
                        }),
                        2 => Some(BusAction::Write {
                            address: sp,
                            value: lo,
                        }),
                        _ => None,
                    }
                }
            }

            // Conditional return: fetch + internal + (if taken: read_lo + read_hi + internal)
            Phase::CondReturn {
                fetches,
                fetch_pc,
                taken,
                sp,
                action,
            } => {
                let sp = *sp;
                let taken = *taken;
                if step < *fetches {
                    Some(BusAction::Read {
                        address: fetch_pc.wrapping_add(step as u16),
                    })
                } else {
                    let post_fetch = step - *fetches;
                    match post_fetch {
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
            }

            // Interrupt dispatch: 5 M-cycles, no fetches
            // Internal, Internal, Write [SP+1] (hi), Write [SP] (lo), Internal
            Phase::InterruptDispatch { sp, pc_hi, pc_lo } => {
                let sp = *sp;
                let pc_hi = *pc_hi;
                let pc_lo = *pc_lo;
                match step {
                    0 => Some(BusAction::Internal),
                    1 => Some(BusAction::Internal),
                    2 => Some(BusAction::Write {
                        address: sp.wrapping_add(1),
                        value: pc_hi,
                    }),
                    3 => Some(BusAction::Write {
                        address: sp,
                        value: pc_lo,
                    }),
                    4 => Some(BusAction::Internal),
                    _ => None,
                }
            }
        }
    }
}
