use super::super::{
    commit::Commit,
    flags::Flags,
    instructions::{
        jump, Address, Arithmetic, Arithmetic16, Arithmetic8, BitFlag, BitShift, Bitwise, Jump,
        Load, Source16, Source8, Stack, Target16, Target8,
    },
    registers::Register16,
    Cpu,
};
use super::{AluOp, Phase, PopAction, ReadAction, RmwOp};

impl Cpu {
    fn resolve_address(cpu: &Cpu, address: &Address) -> u16 {
        match address {
            Address::Fixed(addr) => *addr,
            Address::Relative(_) => unreachable!(),
            Address::High(offset) => 0xff00 + *offset as u16,
            Address::HighPlusC => 0xff00 + cpu.c as u16,
            Address::Dereference(reg) => cpu.get_register16(*reg),
            Address::DereferenceHlAndIncrement => cpu.get_register16(Register16::Hl),
            Address::DereferenceHlAndDecrement => cpu.get_register16(Register16::Hl),
        }
    }

    fn hl_post_delta(address: &Address) -> i16 {
        match address {
            Address::DereferenceHlAndIncrement => 1,
            Address::DereferenceHlAndDecrement => -1,
            _ => 0,
        }
    }

    fn resolve_jump(cpu: &Cpu, location: &jump::Location) -> u16 {
        match location {
            jump::Location::Address(address) => match address {
                Address::Fixed(addr) => *addr,
                Address::Relative(offset) => match offset {
                    0.. => cpu.bus_counter + offset.unsigned_abs() as u16,
                    ..0 => cpu.bus_counter - offset.unsigned_abs() as u16,
                },
                _ => unreachable!(),
            },
            jump::Location::RegisterHl => cpu.get_register16(Register16::Hl),
        }
    }

    fn check_condition(cpu: &Cpu, condition: &Option<jump::Condition>) -> bool {
        if let Some(jump::Condition(flag, value)) = condition {
            cpu.flags.contains(flag.clone().into()) == *value
        } else {
            true
        }
    }

    pub(super) fn build_load(cpu: &mut Cpu, load: &Load) -> (Phase, Commit) {
        match load {
            Load::Load8(target, source) => match (target, source) {
                (Target8::Register(treg), Source8::Constant(val)) => (
                    Phase::Empty,
                    Commit::LoadR8 {
                        reg: *treg,
                        value: *val,
                    },
                ),
                (Target8::Register(treg), Source8::Register(sreg)) => {
                    let value = cpu.get_register8(*sreg);
                    (Phase::Empty, Commit::LoadR8 { reg: *treg, value })
                }
                (Target8::Register(treg), Source8::Memory(address)) => {
                    let addr = Self::resolve_address(cpu, address);
                    let delta = Self::hl_post_delta(address);
                    let action = if delta != 0 {
                        ReadAction::LoadRegisterHlPost(*treg, delta)
                    } else {
                        ReadAction::LoadRegister(*treg)
                    };
                    (
                        Phase::ReadOp {
                            address: addr,
                            action,
                        },
                        Commit::NoOperation,
                    )
                }
                (Target8::Memory(address), source) => {
                    let addr = Self::resolve_address(cpu, address);
                    let delta = Self::hl_post_delta(address);
                    let value = match source {
                        Source8::Constant(v) => *v,
                        Source8::Register(r) => cpu.get_register8(*r),
                        Source8::Memory(_) => unreachable!(),
                    };
                    (
                        Phase::WriteOp {
                            address: addr,
                            value,
                            hl_post: delta,
                        },
                        Commit::NoOperation,
                    )
                }
            },
            Load::Load16(target, source) => match (target, source) {
                (Target16::Register(reg), Source16::Constant(val)) => (
                    Phase::Empty,
                    Commit::LoadR16 {
                        reg: *reg,
                        value: *val,
                    },
                ),
                (Target16::Register(reg), Source16::Register(sreg)) => {
                    // LD SP,HL: decode-edge register write (multi-cycle
                    // instruction — Phase::InternalOamBug follows).
                    let value = cpu.get_register16(*sreg);
                    cpu.set_register16(*reg, value);
                    (
                        Phase::InternalOamBug { address: value },
                        Commit::NoOperation,
                    )
                }
                (Target16::Register(reg), Source16::StackPointerWithOffset(offset)) => {
                    // LD HL,SP+e8: decode-edge register + flags write
                    // (multi-cycle — Phase::InternalOp follows).
                    let sp = cpu.stack_pointer;
                    let offset_u8 = *offset as u8;
                    let result = sp.wrapping_add(*offset as i16 as u16);
                    cpu.flags.remove(Flags::ZERO);
                    cpu.flags.remove(Flags::NEGATIVE);
                    cpu.flags.set(
                        Flags::HALF_CARRY,
                        (sp & 0xf) + (offset_u8 as u16 & 0xf) > 0xf,
                    );
                    cpu.flags
                        .set(Flags::CARRY, (sp & 0xff) + (offset_u8 as u16 & 0xff) > 0xff);
                    cpu.set_register16(*reg, result);
                    (Phase::InternalOp { count: 1 }, Commit::NoOperation)
                }
                (Target16::Memory(address), source) => {
                    let addr = Self::resolve_address(cpu, address);
                    let value = match source {
                        Source16::Constant(v) => *v,
                        Source16::Register(r) => cpu.get_register16(*r),
                        Source16::StackPointerWithOffset(_) => unreachable!(),
                    };
                    (
                        Phase::Write16 {
                            address: addr,
                            lo: (value & 0xff) as u8,
                            hi: (value >> 8) as u8,
                        },
                        Commit::NoOperation,
                    )
                }
            },
        }
    }

    pub(super) fn build_arithmetic(cpu: &mut Cpu, arith: &Arithmetic) -> (Phase, Commit) {
        match arith {
            Arithmetic::Arithmetic8(a8) => match a8 {
                Arithmetic8::Increment(target) => match target {
                    Target8::Register(reg) => (Phase::Empty, Commit::IncR8 { reg: *reg }),
                    Target8::Memory(address) => {
                        let addr = Self::resolve_address(cpu, address);
                        (
                            Phase::ReadModifyWrite {
                                address: addr,
                                op: RmwOp::Increment,
                            },
                            Commit::NoOperation,
                        )
                    }
                },
                Arithmetic8::Decrement(target) => match target {
                    Target8::Register(reg) => (Phase::Empty, Commit::DecR8 { reg: *reg }),
                    Target8::Memory(address) => {
                        let addr = Self::resolve_address(cpu, address);
                        (
                            Phase::ReadModifyWrite {
                                address: addr,
                                op: RmwOp::Decrement,
                            },
                            Commit::NoOperation,
                        )
                    }
                },
                Arithmetic8::AddA(source) => Self::build_alu_source(cpu, source, AluOp::Add),
                Arithmetic8::SubtractA(source) => Self::build_alu_source(cpu, source, AluOp::Sub),
                Arithmetic8::AddACarry(source) => {
                    let carry = if cpu.flags.contains(Flags::CARRY) {
                        1
                    } else {
                        0
                    };
                    Self::build_alu_source(cpu, source, AluOp::Adc { carry })
                }
                Arithmetic8::SubtractACarry(source) => {
                    let carry = if cpu.flags.contains(Flags::CARRY) {
                        1
                    } else {
                        0
                    };
                    Self::build_alu_source(cpu, source, AluOp::Sbc { carry })
                }
                Arithmetic8::CompareA(source) => Self::build_alu_source(cpu, source, AluOp::Cp),
            },
            Arithmetic::Arithmetic16(a16) => match a16 {
                Arithmetic16::Increment(reg) => {
                    // Decode-edge register write (multi-cycle — Phase::InternalOamBug
                    // shows the OLD address on the bus).
                    let old = cpu.get_register16(*reg);
                    cpu.set_register16(*reg, old.wrapping_add(1));
                    (Phase::InternalOamBug { address: old }, Commit::NoOperation)
                }
                Arithmetic16::Decrement(reg) => {
                    let old = cpu.get_register16(*reg);
                    cpu.set_register16(*reg, old.wrapping_sub(1));
                    (Phase::InternalOamBug { address: old }, Commit::NoOperation)
                }
                Arithmetic16::AddHl(reg) => {
                    // Decode-edge HL + flags write (multi-cycle).
                    let value = cpu.get_register16(*reg);
                    let hl = cpu.get_register16(Register16::Hl);
                    cpu.flags.remove(Flags::NEGATIVE);
                    cpu.flags.set(
                        Flags::HALF_CARRY,
                        (hl & 0xfff) as u32 + (value & 0xfff) as u32 > 0xfff,
                    );
                    cpu.flags
                        .set(Flags::CARRY, hl as u32 + value as u32 > 0xffff);
                    cpu.set_register16(Register16::Hl, hl.wrapping_add(value));
                    (Phase::InternalOp { count: 1 }, Commit::NoOperation)
                }
            },
        }
    }

    fn build_alu_source(cpu: &mut Cpu, source: &Source8, op: AluOp) -> (Phase, Commit) {
        match source {
            Source8::Constant(val) => (Phase::Empty, Commit::AluA { op, value: *val }),
            Source8::Register(reg) => {
                let value = cpu.get_register8(*reg);
                (Phase::Empty, Commit::AluA { op, value })
            }
            Source8::Memory(address) => {
                let addr = Self::resolve_address(cpu, address);
                (
                    Phase::ReadOp {
                        address: addr,
                        action: ReadAction::AluA(op),
                    },
                    Commit::NoOperation,
                )
            }
        }
    }

    pub(super) fn build_bitwise(cpu: &mut Cpu, bw: &Bitwise) -> (Phase, Commit) {
        match bw {
            Bitwise::AndA(source) => Self::build_alu_source(cpu, source, AluOp::And),
            Bitwise::OrA(source) => Self::build_alu_source(cpu, source, AluOp::Or),
            Bitwise::XorA(source) => Self::build_alu_source(cpu, source, AluOp::Xor),
            Bitwise::ComplementA => (Phase::Empty, Commit::ComplementA),
        }
    }

    pub(super) fn build_bit_shift(cpu: &mut Cpu, bs: &BitShift) -> (Phase, Commit) {
        match bs {
            BitShift::RotateA(direction, carry) => (
                Phase::Empty,
                Commit::RotateAccumulator {
                    direction: direction.clone(),
                    carry: carry.clone(),
                },
            ),
            BitShift::Rotate(direction, carry, target) => match target {
                Target8::Register(reg) => (
                    Phase::Empty,
                    Commit::RotateReg {
                        reg: *reg,
                        direction: direction.clone(),
                        carry: carry.clone(),
                    },
                ),
                Target8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    (
                        Phase::ReadModifyWrite {
                            address: addr,
                            op: RmwOp::Rotate(direction.clone(), carry.clone()),
                        },
                        Commit::NoOperation,
                    )
                }
            },
            BitShift::ShiftArithmetical(direction, target) => match target {
                Target8::Register(reg) => (
                    Phase::Empty,
                    Commit::ShiftArithmetical {
                        reg: *reg,
                        direction: direction.clone(),
                    },
                ),
                Target8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    (
                        Phase::ReadModifyWrite {
                            address: addr,
                            op: RmwOp::ShiftArithmetical(direction.clone()),
                        },
                        Commit::NoOperation,
                    )
                }
            },
            BitShift::ShiftRightLogical(target) => match target {
                Target8::Register(reg) => (Phase::Empty, Commit::ShiftRightLogical { reg: *reg }),
                Target8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    (
                        Phase::ReadModifyWrite {
                            address: addr,
                            op: RmwOp::ShiftRightLogical,
                        },
                        Commit::NoOperation,
                    )
                }
            },
            BitShift::Swap(target) => match target {
                Target8::Register(reg) => (Phase::Empty, Commit::SwapReg { reg: *reg }),
                Target8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    (
                        Phase::ReadModifyWrite {
                            address: addr,
                            op: RmwOp::Swap,
                        },
                        Commit::NoOperation,
                    )
                }
            },
        }
    }

    pub(super) fn build_bit_flag(cpu: &Cpu, bf: &BitFlag) -> (Phase, Commit) {
        match bf {
            BitFlag::Check(bit, source) => match source {
                Source8::Register(reg) => (
                    Phase::Empty,
                    Commit::BitTest {
                        bit: *bit,
                        reg: *reg,
                    },
                ),
                Source8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    (
                        Phase::ReadOp {
                            address: addr,
                            action: ReadAction::BitTest(*bit),
                        },
                        Commit::NoOperation,
                    )
                }
                Source8::Constant(_) => unreachable!(),
            },
            BitFlag::Set(bit, target) => match target {
                Target8::Register(reg) => (
                    Phase::Empty,
                    Commit::BitSet {
                        bit: *bit,
                        reg: *reg,
                    },
                ),
                Target8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    (
                        Phase::ReadModifyWrite {
                            address: addr,
                            op: RmwOp::BitSet(*bit),
                        },
                        Commit::NoOperation,
                    )
                }
            },
            BitFlag::Unset(bit, target) => match target {
                Target8::Register(reg) => (
                    Phase::Empty,
                    Commit::BitReset {
                        bit: *bit,
                        reg: *reg,
                    },
                ),
                Target8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    (
                        Phase::ReadModifyWrite {
                            address: addr,
                            op: RmwOp::BitReset(*bit),
                        },
                        Commit::NoOperation,
                    )
                }
            },
        }
    }

    pub(super) fn build_jump(cpu: &mut Cpu, j: &Jump) -> (Phase, Commit) {
        match j {
            Jump::Jump(condition, location) => {
                let is_relative = matches!(location, jump::Location::Address(Address::Relative(_)));
                let address = Self::resolve_jump(cpu, location);
                let taken = Self::check_condition(cpu, condition);
                if matches!(location, jump::Location::RegisterHl) {
                    // JP HL: no internal M-cycle. The bus address driver
                    // mux selects HL combinationally on IR for the next
                    // fetch cell — modelled by writing bus_counter at
                    // decode time so the trailing FetchOverlap's Read
                    // uses HL.
                    if taken {
                        cpu.bus_counter = address;
                        cpu.pc = cpu.bus_counter;
                    }
                    (Phase::Empty, Commit::NoOperation)
                } else if is_relative && taken {
                    // JR taken: decode-edge bus_counter + pc write
                    // (multi-cycle — Phase::InternalOamBug shows target
                    // on bus).
                    cpu.bus_counter = address;
                    cpu.pc = cpu.bus_counter;
                    (Phase::InternalOamBug { address }, Commit::NoOperation)
                } else {
                    // JP nn / JP cc,nn: defer PC update to the internal
                    // M-cycle.
                    (
                        Phase::CondJump {
                            taken,
                            target: address,
                        },
                        Commit::NoOperation,
                    )
                }
            }
            Jump::Call(condition, location) => {
                let address = Self::resolve_jump(cpu, location);
                let taken = Self::check_condition(cpu, condition);
                if taken {
                    let pc = cpu.pc;
                    let pc_hi = (pc >> 8) as u8;
                    let pc_lo = (pc & 0xff) as u8;
                    let sp = cpu.stack_pointer;
                    cpu.bus_counter = address;
                    cpu.pc = cpu.bus_counter;
                    (
                        Phase::CondCall {
                            taken: true,
                            sp,
                            hi: pc_hi,
                            lo: pc_lo,
                        },
                        Commit::NoOperation,
                    )
                } else {
                    (
                        Phase::CondCall {
                            taken: false,
                            sp: 0,
                            hi: 0,
                            lo: 0,
                        },
                        Commit::NoOperation,
                    )
                }
            }
            Jump::Return(condition) => {
                let has_condition = condition.is_some();
                let taken = Self::check_condition(cpu, condition);
                let phase = if has_condition {
                    Phase::CondReturn {
                        taken,
                        sp: cpu.stack_pointer,
                        action: PopAction::SetPc,
                    }
                } else {
                    Phase::Pop {
                        sp: cpu.stack_pointer,
                        action: PopAction::SetPc,
                    }
                };
                (phase, Commit::NoOperation)
            }
            Jump::ReturnAndEnableInterrupts => (
                Phase::Pop {
                    sp: cpu.stack_pointer,
                    action: PopAction::SetPcEnableInterrupts,
                },
                Commit::NoOperation,
            ),
            Jump::Restart(address) => {
                let pc = cpu.pc;
                let pc_hi = (pc >> 8) as u8;
                let pc_lo = (pc & 0xff) as u8;
                let sp = cpu.stack_pointer;
                cpu.bus_counter = *address as u16;
                cpu.pc = cpu.bus_counter;
                (
                    Phase::Push {
                        sp,
                        hi: pc_hi,
                        lo: pc_lo,
                    },
                    Commit::NoOperation,
                )
            }
        }
    }

    pub(super) fn build_stack(cpu: &mut Cpu, s: &Stack) -> (Phase, Commit) {
        match s {
            Stack::Push(register) => {
                let value = cpu.get_register16(*register);
                let hi = (value >> 8) as u8;
                let lo = (value & 0xff) as u8;
                let sp = cpu.stack_pointer;
                (Phase::Push { sp, hi, lo }, Commit::NoOperation)
            }
            Stack::Pop(register) => (
                Phase::Pop {
                    sp: cpu.stack_pointer,
                    action: PopAction::SetRegister(*register),
                },
                Commit::NoOperation,
            ),
            Stack::Adjust(offset) => {
                // ADD SP,e8: decode-edge SP + flags write (multi-cycle —
                // two internal M-cycles follow).
                let sp = cpu.stack_pointer;
                let offset_u8 = *offset as u8;
                cpu.stack_pointer = sp.wrapping_add(*offset as i16 as u16);
                cpu.flags.remove(Flags::ZERO);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.set(
                    Flags::HALF_CARRY,
                    (sp & 0xf) + (offset_u8 as u16 & 0xf) > 0xf,
                );
                cpu.flags
                    .set(Flags::CARRY, (sp & 0xff) + (offset_u8 as u16 & 0xff) > 0xff);
                (Phase::InternalOp { count: 2 }, Commit::NoOperation)
            }
        }
    }
}
