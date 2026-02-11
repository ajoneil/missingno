use super::super::{
    Cpu,
    flags::Flags,
    instructions::{
        Address, Arithmetic, Arithmetic8, Arithmetic16, BitFlag, BitShift, Bitwise, Jump, Load,
        Source8, Source16, Stack, Target8, Target16, bit_shift::Direction, jump,
    },
    registers::Register16,
};
use super::{AluOp, InstructionStepper, Phase, PopAction, ReadAction, RmwOp};

impl InstructionStepper {
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
                    0.. => cpu.program_counter + offset.unsigned_abs() as u16,
                    ..0 => cpu.program_counter - offset.unsigned_abs() as u16,
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

    pub(super) fn build_load(cpu: &mut Cpu, load: &Load, fetches: u8, fetch_pc: u16) -> Phase {
        match load {
            Load::Load8(target, source) => match (target, source) {
                (Target8::Register(treg), Source8::Constant(val)) => {
                    cpu.set_register8(*treg, *val);
                    Phase::FetchOnly { fetches, fetch_pc }
                }
                (Target8::Register(treg), Source8::Register(sreg)) => {
                    cpu.set_register8(*treg, cpu.get_register8(*sreg));
                    Phase::FetchOnly { fetches, fetch_pc }
                }
                (Target8::Register(treg), Source8::Memory(address)) => {
                    let addr = Self::resolve_address(cpu, address);
                    let delta = Self::hl_post_delta(address);
                    let action = if delta != 0 {
                        ReadAction::LoadRegisterHlPost(*treg, delta)
                    } else {
                        ReadAction::LoadRegister(*treg)
                    };
                    Phase::ReadOp {
                        fetches,
                        fetch_pc,
                        address: addr,
                        action,
                    }
                }
                (Target8::Memory(address), source) => {
                    let addr = Self::resolve_address(cpu, address);
                    let delta = Self::hl_post_delta(address);
                    let value = match source {
                        Source8::Constant(v) => *v,
                        Source8::Register(r) => cpu.get_register8(*r),
                        Source8::Memory(_) => unreachable!(),
                    };
                    Phase::WriteOp {
                        fetches,
                        fetch_pc,
                        address: addr,
                        value,
                        hl_post: delta,
                    }
                }
            },
            Load::Load16(target, source) => match (target, source) {
                (Target16::Register(reg), Source16::Constant(val)) => {
                    cpu.set_register16(*reg, *val);
                    Phase::FetchOnly { fetches, fetch_pc }
                }
                (Target16::Register(reg), Source16::Register(sreg)) => {
                    cpu.set_register16(*reg, cpu.get_register16(*sreg));
                    Phase::InternalOp {
                        fetches,
                        fetch_pc,
                        count: 1,
                    }
                }
                (Target16::Register(reg), Source16::StackPointerWithOffset(offset)) => {
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
                    Phase::InternalOp {
                        fetches,
                        fetch_pc,
                        count: 1,
                    }
                }
                (Target16::Memory(address), source) => {
                    let addr = Self::resolve_address(cpu, address);
                    let value = match source {
                        Source16::Constant(v) => *v,
                        Source16::Register(r) => cpu.get_register16(*r),
                        Source16::StackPointerWithOffset(_) => unreachable!(),
                    };
                    Phase::Write16 {
                        fetches,
                        fetch_pc,
                        address: addr,
                        lo: (value & 0xff) as u8,
                        hi: (value >> 8) as u8,
                    }
                }
            },
        }
    }

    pub(super) fn build_arithmetic(
        cpu: &mut Cpu,
        arith: &Arithmetic,
        fetches: u8,
        fetch_pc: u16,
    ) -> Phase {
        match arith {
            Arithmetic::Arithmetic8(a8) => match a8 {
                Arithmetic8::Increment(target) => match target {
                    Target8::Register(reg) => {
                        let val = cpu.get_register8(*reg);
                        let result = val.wrapping_add(1);
                        cpu.flags.set(Flags::ZERO, result == 0);
                        cpu.flags.remove(Flags::NEGATIVE);
                        cpu.flags.set(Flags::HALF_CARRY, result & 0b1111 == 0b0000);
                        cpu.set_register8(*reg, result);
                        Phase::FetchOnly { fetches, fetch_pc }
                    }
                    Target8::Memory(address) => {
                        let addr = Self::resolve_address(cpu, address);
                        Phase::ReadModifyWrite {
                            fetches,
                            fetch_pc,
                            address: addr,
                            op: RmwOp::Increment,
                        }
                    }
                },
                Arithmetic8::Decrement(target) => match target {
                    Target8::Register(reg) => {
                        let val = cpu.get_register8(*reg);
                        let result = val.wrapping_sub(1);
                        cpu.flags.set(Flags::ZERO, result == 0);
                        cpu.flags.insert(Flags::NEGATIVE);
                        cpu.flags.set(Flags::HALF_CARRY, result & 0b1111 == 0b1111);
                        cpu.set_register8(*reg, result);
                        Phase::FetchOnly { fetches, fetch_pc }
                    }
                    Target8::Memory(address) => {
                        let addr = Self::resolve_address(cpu, address);
                        Phase::ReadModifyWrite {
                            fetches,
                            fetch_pc,
                            address: addr,
                            op: RmwOp::Decrement,
                        }
                    }
                },
                Arithmetic8::AddA(source) => {
                    Self::build_alu_source(cpu, source, AluOp::Add, fetches, fetch_pc)
                }
                Arithmetic8::SubtractA(source) => {
                    Self::build_alu_source(cpu, source, AluOp::Sub, fetches, fetch_pc)
                }
                Arithmetic8::AddACarry(source) => {
                    let carry = if cpu.flags.contains(Flags::CARRY) {
                        1
                    } else {
                        0
                    };
                    Self::build_alu_source(cpu, source, AluOp::Adc { carry }, fetches, fetch_pc)
                }
                Arithmetic8::SubtractACarry(source) => {
                    let carry = if cpu.flags.contains(Flags::CARRY) {
                        1
                    } else {
                        0
                    };
                    Self::build_alu_source(cpu, source, AluOp::Sbc { carry }, fetches, fetch_pc)
                }
                Arithmetic8::CompareA(source) => {
                    Self::build_alu_source(cpu, source, AluOp::Cp, fetches, fetch_pc)
                }
            },
            Arithmetic::Arithmetic16(a16) => match a16 {
                Arithmetic16::Increment(reg) => {
                    cpu.set_register16(*reg, cpu.get_register16(*reg).wrapping_add(1));
                    Phase::InternalOp {
                        fetches,
                        fetch_pc,
                        count: 1,
                    }
                }
                Arithmetic16::Decrement(reg) => {
                    cpu.set_register16(*reg, cpu.get_register16(*reg).wrapping_sub(1));
                    Phase::InternalOp {
                        fetches,
                        fetch_pc,
                        count: 1,
                    }
                }
                Arithmetic16::AddHl(reg) => {
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
                    Phase::InternalOp {
                        fetches,
                        fetch_pc,
                        count: 1,
                    }
                }
            },
        }
    }

    fn build_alu_source(
        cpu: &mut Cpu,
        source: &Source8,
        op: AluOp,
        fetches: u8,
        fetch_pc: u16,
    ) -> Phase {
        match source {
            Source8::Constant(val) => {
                Self::apply_alu(cpu, &op, *val);
                Phase::FetchOnly { fetches, fetch_pc }
            }
            Source8::Register(reg) => {
                let val = cpu.get_register8(*reg);
                Self::apply_alu(cpu, &op, val);
                Phase::FetchOnly { fetches, fetch_pc }
            }
            Source8::Memory(address) => {
                let addr = Self::resolve_address(cpu, address);
                Phase::ReadOp {
                    fetches,
                    fetch_pc,
                    address: addr,
                    action: ReadAction::AluA(op),
                }
            }
        }
    }

    pub(super) fn build_bitwise(cpu: &mut Cpu, bw: &Bitwise, fetches: u8, fetch_pc: u16) -> Phase {
        match bw {
            Bitwise::AndA(source) => {
                Self::build_alu_source(cpu, source, AluOp::And, fetches, fetch_pc)
            }
            Bitwise::OrA(source) => {
                Self::build_alu_source(cpu, source, AluOp::Or, fetches, fetch_pc)
            }
            Bitwise::XorA(source) => {
                Self::build_alu_source(cpu, source, AluOp::Xor, fetches, fetch_pc)
            }
            Bitwise::ComplementA => {
                cpu.a = !cpu.a;
                cpu.flags.insert(Flags::NEGATIVE);
                cpu.flags.insert(Flags::HALF_CARRY);
                Phase::FetchOnly { fetches, fetch_pc }
            }
        }
    }

    pub(super) fn build_bit_shift(
        cpu: &mut Cpu,
        bs: &BitShift,
        fetches: u8,
        fetch_pc: u16,
    ) -> Phase {
        match bs {
            BitShift::RotateA(direction, carry) => {
                let (new_value, new_carry) = Self::rotate(cpu, cpu.a, direction, carry);
                cpu.flags = if new_carry {
                    Flags::CARRY
                } else {
                    Flags::empty()
                };
                cpu.a = new_value;
                Phase::FetchOnly { fetches, fetch_pc }
            }
            BitShift::Rotate(direction, carry, target) => match target {
                Target8::Register(reg) => {
                    let val = cpu.get_register8(*reg);
                    let (new_value, new_carry) = Self::rotate(cpu, val, direction, carry);
                    cpu.flags.set(Flags::ZERO, new_value == 0);
                    cpu.flags.set(Flags::CARRY, new_carry);
                    cpu.flags.remove(Flags::NEGATIVE);
                    cpu.flags.remove(Flags::HALF_CARRY);
                    cpu.set_register8(*reg, new_value);
                    Phase::FetchOnly { fetches, fetch_pc }
                }
                Target8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    Phase::ReadModifyWrite {
                        fetches,
                        fetch_pc,
                        address: addr,
                        op: RmwOp::Rotate(direction.clone(), carry.clone()),
                    }
                }
            },
            BitShift::ShiftArithmetical(direction, target) => match target {
                Target8::Register(reg) => {
                    let val = cpu.get_register8(*reg);
                    let new_value = match direction {
                        Direction::Left => {
                            cpu.flags.set(Flags::CARRY, val & 0b1000_0000 != 0);
                            val << 1
                        }
                        Direction::Right => {
                            cpu.flags.set(Flags::CARRY, val & 0b0000_0001 != 0);
                            val >> 1 | (val & 0b1000_0000)
                        }
                    };
                    cpu.flags.remove(Flags::NEGATIVE);
                    cpu.flags.remove(Flags::HALF_CARRY);
                    cpu.flags.set(Flags::ZERO, new_value == 0);
                    cpu.set_register8(*reg, new_value);
                    Phase::FetchOnly { fetches, fetch_pc }
                }
                Target8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    Phase::ReadModifyWrite {
                        fetches,
                        fetch_pc,
                        address: addr,
                        op: RmwOp::ShiftArithmetical(direction.clone()),
                    }
                }
            },
            BitShift::ShiftRightLogical(target) => match target {
                Target8::Register(reg) => {
                    let val = cpu.get_register8(*reg);
                    let new_value = val >> 1;
                    cpu.flags.set(Flags::CARRY, val & 0b0000_0001 != 0);
                    cpu.flags.remove(Flags::NEGATIVE);
                    cpu.flags.remove(Flags::HALF_CARRY);
                    cpu.flags.set(Flags::ZERO, new_value == 0);
                    cpu.set_register8(*reg, new_value);
                    Phase::FetchOnly { fetches, fetch_pc }
                }
                Target8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    Phase::ReadModifyWrite {
                        fetches,
                        fetch_pc,
                        address: addr,
                        op: RmwOp::ShiftRightLogical,
                    }
                }
            },
            BitShift::Swap(target) => match target {
                Target8::Register(reg) => {
                    let val = cpu.get_register8(*reg);
                    let new_value = val << 4 | (val >> 4 & 0xf);
                    cpu.flags = if new_value == 0 {
                        Flags::ZERO
                    } else {
                        Flags::empty()
                    };
                    cpu.set_register8(*reg, new_value);
                    Phase::FetchOnly { fetches, fetch_pc }
                }
                Target8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    Phase::ReadModifyWrite {
                        fetches,
                        fetch_pc,
                        address: addr,
                        op: RmwOp::Swap,
                    }
                }
            },
        }
    }

    pub(super) fn build_bit_flag(cpu: &mut Cpu, bf: &BitFlag, fetches: u8, fetch_pc: u16) -> Phase {
        match bf {
            BitFlag::Check(bit, source) => match source {
                Source8::Register(reg) => {
                    let val = cpu.get_register8(*reg);
                    cpu.flags.set(Flags::ZERO, val & (1 << bit) == 0);
                    cpu.flags.remove(Flags::NEGATIVE);
                    cpu.flags.insert(Flags::HALF_CARRY);
                    Phase::FetchOnly { fetches, fetch_pc }
                }
                Source8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    Phase::ReadOp {
                        fetches,
                        fetch_pc,
                        address: addr,
                        action: ReadAction::BitTest(*bit),
                    }
                }
                Source8::Constant(_) => unreachable!(),
            },
            BitFlag::Set(bit, target) => match target {
                Target8::Register(reg) => {
                    let val = cpu.get_register8(*reg);
                    cpu.set_register8(*reg, val | (1 << bit));
                    Phase::FetchOnly { fetches, fetch_pc }
                }
                Target8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    Phase::ReadModifyWrite {
                        fetches,
                        fetch_pc,
                        address: addr,
                        op: RmwOp::BitSet(*bit),
                    }
                }
            },
            BitFlag::Unset(bit, target) => match target {
                Target8::Register(reg) => {
                    let val = cpu.get_register8(*reg);
                    cpu.set_register8(*reg, val & !(1 << bit));
                    Phase::FetchOnly { fetches, fetch_pc }
                }
                Target8::Memory(address) => {
                    let addr = Self::resolve_address(cpu, address);
                    Phase::ReadModifyWrite {
                        fetches,
                        fetch_pc,
                        address: addr,
                        op: RmwOp::BitReset(*bit),
                    }
                }
            },
        }
    }

    pub(super) fn build_jump(cpu: &mut Cpu, j: &Jump, fetches: u8, fetch_pc: u16) -> Phase {
        match j {
            Jump::Jump(condition, location) => {
                let address = Self::resolve_jump(cpu, location);
                let taken = Self::check_condition(cpu, condition);
                if taken {
                    cpu.program_counter = address;
                }
                if matches!(location, jump::Location::RegisterHl) {
                    Phase::FetchOnly { fetches, fetch_pc }
                } else {
                    Phase::CondJump {
                        fetches,
                        fetch_pc,
                        taken,
                    }
                }
            }
            Jump::Call(condition, location) => {
                let address = Self::resolve_jump(cpu, location);
                let taken = Self::check_condition(cpu, condition);
                if taken {
                    let pc = cpu.program_counter;
                    let pc_hi = (pc >> 8) as u8;
                    let pc_lo = (pc & 0xff) as u8;
                    cpu.stack_pointer = cpu.stack_pointer.wrapping_sub(2);
                    cpu.program_counter = address;
                    Phase::CondCall {
                        fetches,
                        fetch_pc,
                        taken: true,
                        sp: cpu.stack_pointer,
                        hi: pc_hi,
                        lo: pc_lo,
                    }
                } else {
                    Phase::CondCall {
                        fetches,
                        fetch_pc,
                        taken: false,
                        sp: 0,
                        hi: 0,
                        lo: 0,
                    }
                }
            }
            Jump::Return(condition) => {
                let has_condition = condition.is_some();
                let taken = Self::check_condition(cpu, condition);
                if has_condition {
                    Phase::CondReturn {
                        fetches,
                        fetch_pc,
                        taken,
                        sp: cpu.stack_pointer,
                        action: PopAction::SetPc,
                    }
                } else {
                    Phase::Pop {
                        fetches,
                        fetch_pc,
                        sp: cpu.stack_pointer,
                        action: PopAction::SetPc,
                    }
                }
            }
            Jump::ReturnAndEnableInterrupts => Phase::Pop {
                fetches,
                fetch_pc,
                sp: cpu.stack_pointer,
                action: PopAction::SetPcEnableInterrupts,
            },
            Jump::Restart(address) => {
                let pc = cpu.program_counter;
                let pc_hi = (pc >> 8) as u8;
                let pc_lo = (pc & 0xff) as u8;
                cpu.stack_pointer = cpu.stack_pointer.wrapping_sub(2);
                cpu.program_counter = *address as u16;
                Phase::Push {
                    fetches,
                    fetch_pc,
                    sp: cpu.stack_pointer,
                    hi: pc_hi,
                    lo: pc_lo,
                }
            }
        }
    }

    pub(super) fn build_stack(cpu: &mut Cpu, s: &Stack, fetches: u8, fetch_pc: u16) -> Phase {
        match s {
            Stack::Push(register) => {
                let value = cpu.get_register16(*register);
                let hi = (value >> 8) as u8;
                let lo = (value & 0xff) as u8;
                cpu.stack_pointer = cpu.stack_pointer.wrapping_sub(2);
                Phase::Push {
                    fetches,
                    fetch_pc,
                    sp: cpu.stack_pointer,
                    hi,
                    lo,
                }
            }
            Stack::Pop(register) => Phase::Pop {
                fetches,
                fetch_pc,
                sp: cpu.stack_pointer,
                action: PopAction::SetRegister(*register),
            },
            Stack::Adjust(offset) => {
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
                Phase::InternalOp {
                    fetches,
                    fetch_pc,
                    count: 2,
                }
            }
        }
    }
}
