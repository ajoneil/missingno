use super::super::{
    Cpu, EiDelay, InterruptMasterEnable,
    flags::Flags,
    instructions::bit_shift::{Carry, Direction},
    instructions::{CarryFlag, Interrupt as InterruptInstruction},
    registers::Register16,
};
use super::{AluOp, PopAction, Processor, ReadAction, RmwOp};

impl Processor {
    pub(super) fn apply_daa(cpu: &mut Cpu) {
        let value = if cpu.flags.contains(Flags::NEGATIVE) {
            let mut adj = 0u8;
            if cpu.flags.contains(Flags::HALF_CARRY) {
                adj += 0x6;
            }
            if cpu.flags.contains(Flags::CARRY) {
                adj += 0x60;
            }
            cpu.a.wrapping_sub(adj)
        } else {
            let mut adj = 0u8;
            if cpu.flags.contains(Flags::HALF_CARRY) || cpu.a & 0xf > 0x9 {
                adj += 0x6;
            }
            if cpu.flags.contains(Flags::CARRY) || cpu.a > 0x99 {
                adj += 0x60;
                cpu.flags.insert(Flags::CARRY);
            }
            cpu.a.wrapping_add(adj)
        };
        cpu.flags.set(Flags::ZERO, value == 0);
        cpu.flags.remove(Flags::HALF_CARRY);
        cpu.a = value;
    }

    pub(super) fn apply_carry_flag(cpu: &mut Cpu, cf: &CarryFlag) {
        match cf {
            CarryFlag::Complement => {
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.remove(Flags::HALF_CARRY);
                cpu.flags.toggle(Flags::CARRY);
            }
            CarryFlag::Set => {
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.remove(Flags::HALF_CARRY);
                cpu.flags.insert(Flags::CARRY);
            }
        }
    }

    pub(super) fn apply_interrupt_instruction(cpu: &mut Cpu, instr: &InterruptInstruction) {
        match instr {
            InterruptInstruction::Enable => {
                if cpu.interrupt_master_enable != InterruptMasterEnable::Enabled {
                    cpu.ei_delay = Some(EiDelay::Pending);
                }
            }
            InterruptInstruction::Disable => {
                cpu.interrupt_master_enable = InterruptMasterEnable::Disabled;
            }
            InterruptInstruction::Await => {
                cpu.halted = true;
            }
        }
    }

    pub(super) fn apply_read_action(cpu: &mut Cpu, action: &ReadAction, value: u8) {
        match action {
            ReadAction::LoadRegister(reg) => {
                cpu.set_register8(*reg, value);
            }
            ReadAction::LoadRegisterHlPost(reg, delta) => {
                cpu.set_register8(*reg, value);
                let hl = cpu.get_register16(Register16::Hl);
                cpu.set_register16(Register16::Hl, hl.wrapping_add(*delta as u16));
            }
            ReadAction::AluA(op) => {
                Self::apply_alu(cpu, op, value);
            }
            ReadAction::BitTest(bit) => {
                cpu.flags.set(Flags::ZERO, value & (1 << bit) == 0);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.insert(Flags::HALF_CARRY);
            }
        }
    }

    pub(super) fn apply_alu(cpu: &mut Cpu, op: &AluOp, value: u8) {
        match op {
            AluOp::Add => {
                let result = cpu.a.wrapping_add(value);
                cpu.flags.set(Flags::ZERO, result == 0);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags
                    .set(Flags::HALF_CARRY, (cpu.a & 0xf) + (value & 0xf) > 0xf);
                cpu.flags
                    .set(Flags::CARRY, cpu.a as u16 + value as u16 > 0xff);
                cpu.a = result;
            }
            AluOp::Sub => {
                let result = cpu.a.wrapping_sub(value);
                cpu.flags.set(Flags::ZERO, result == 0);
                cpu.flags.insert(Flags::NEGATIVE);
                cpu.flags
                    .set(Flags::HALF_CARRY, (value & 0xf) > (cpu.a & 0xf));
                cpu.flags.set(Flags::CARRY, cpu.a < value);
                cpu.a = result;
            }
            AluOp::Adc { carry } => {
                let c = *carry;
                let result = cpu.a.wrapping_add(value).wrapping_add(c);
                cpu.flags.set(Flags::ZERO, result == 0);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags
                    .set(Flags::HALF_CARRY, (cpu.a & 0xf) + (value & 0xf) + c > 0xf);
                cpu.flags
                    .set(Flags::CARRY, cpu.a as u16 + value as u16 + c as u16 > 0xff);
                cpu.a = result;
            }
            AluOp::Sbc { carry } => {
                let c = *carry;
                let result = cpu.a.wrapping_sub(value).wrapping_sub(c);
                cpu.flags.set(Flags::ZERO, result == 0);
                cpu.flags.insert(Flags::NEGATIVE);
                cpu.flags
                    .set(Flags::HALF_CARRY, (value & 0xf) + c > (cpu.a & 0xf));
                cpu.flags
                    .set(Flags::CARRY, value as u16 + c as u16 > cpu.a as u16);
                cpu.a = result;
            }
            AluOp::Cp => {
                let result = cpu.a.wrapping_sub(value);
                cpu.flags.set(Flags::ZERO, result == 0);
                cpu.flags.insert(Flags::NEGATIVE);
                cpu.flags.set(Flags::HALF_CARRY, value & 0xf > cpu.a & 0xf);
                cpu.flags.set(Flags::CARRY, value > cpu.a);
            }
            AluOp::And => {
                cpu.a &= value;
                cpu.flags.set(Flags::ZERO, cpu.a == 0);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.insert(Flags::HALF_CARRY);
                cpu.flags.remove(Flags::CARRY);
            }
            AluOp::Or => {
                cpu.a |= value;
                cpu.flags.set(Flags::ZERO, cpu.a == 0);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.remove(Flags::HALF_CARRY);
                cpu.flags.remove(Flags::CARRY);
            }
            AluOp::Xor => {
                cpu.a ^= value;
                cpu.flags.set(Flags::ZERO, cpu.a == 0);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.remove(Flags::HALF_CARRY);
                cpu.flags.remove(Flags::CARRY);
            }
        }
    }

    pub(super) fn apply_rmw(cpu: &mut Cpu, op: &RmwOp, value: u8) -> u8 {
        match op {
            RmwOp::Increment => {
                let result = value.wrapping_add(1);
                cpu.flags.set(Flags::ZERO, result == 0);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.set(Flags::HALF_CARRY, result & 0b1111 == 0b0000);
                result
            }
            RmwOp::Decrement => {
                let result = value.wrapping_sub(1);
                cpu.flags.set(Flags::ZERO, result == 0);
                cpu.flags.insert(Flags::NEGATIVE);
                cpu.flags.set(Flags::HALF_CARRY, result & 0b1111 == 0b1111);
                result
            }
            RmwOp::Rotate(direction, carry) => {
                let (new_value, new_carry) = Self::rotate(cpu, value, direction, carry);
                cpu.flags.set(Flags::ZERO, new_value == 0);
                cpu.flags.set(Flags::CARRY, new_carry);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.remove(Flags::HALF_CARRY);
                new_value
            }
            RmwOp::ShiftArithmetical(direction) => {
                let new_value = match direction {
                    Direction::Left => {
                        cpu.flags.set(Flags::CARRY, value & 0b1000_0000 != 0);
                        value << 1
                    }
                    Direction::Right => {
                        cpu.flags.set(Flags::CARRY, value & 0b0000_0001 != 0);
                        value >> 1 | (value & 0b1000_0000)
                    }
                };
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.remove(Flags::HALF_CARRY);
                cpu.flags.set(Flags::ZERO, new_value == 0);
                new_value
            }
            RmwOp::ShiftRightLogical => {
                let new_value = value >> 1;
                cpu.flags.set(Flags::CARRY, value & 0b0000_0001 != 0);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.remove(Flags::HALF_CARRY);
                cpu.flags.set(Flags::ZERO, new_value == 0);
                new_value
            }
            RmwOp::Swap => {
                let new_value = value << 4 | (value >> 4 & 0xf);
                cpu.flags = if new_value == 0 {
                    Flags::ZERO
                } else {
                    Flags::empty()
                };
                new_value
            }
            RmwOp::BitSet(bit) => value | (1 << bit),
            RmwOp::BitReset(bit) => value & !(1 << bit),
        }
    }

    pub(super) fn rotate(cpu: &Cpu, value: u8, direction: &Direction, carry: &Carry) -> (u8, bool) {
        let old_carry = cpu.flags.contains(Flags::CARRY);
        match (direction, carry) {
            (Direction::Left, Carry::SetOnly) => {
                let new_carry = value & 0b1000_0000 != 0;
                (value.rotate_left(1), new_carry)
            }
            (Direction::Right, Carry::SetOnly) => {
                let new_carry = value & 0b0000_0001 != 0;
                (value.rotate_right(1), new_carry)
            }
            (Direction::Left, Carry::Through) => {
                let new_carry = value & 0b1000_0000 != 0;
                let result = (value << 1) | (old_carry as u8);
                (result, new_carry)
            }
            (Direction::Right, Carry::Through) => {
                let new_carry = value & 0b0000_0001 != 0;
                let result = (value >> 1) | ((old_carry as u8) << 7);
                (result, new_carry)
            }
        }
    }

    pub(super) fn apply_pop(cpu: &mut Cpu, action: &PopAction, low: u8, high: u8, sp: u16) {
        cpu.stack_pointer = sp.wrapping_add(2);
        let value = u16::from_le_bytes([low, high]);
        match action {
            PopAction::SetRegister(reg) => {
                cpu.set_register16(*reg, value);
            }
            PopAction::SetPc => {
                cpu.program_counter = value;
            }
            PopAction::SetPcEnableInterrupts => {
                cpu.interrupt_master_enable = InterruptMasterEnable::Enabled;
                cpu.program_counter = value;
            }
        }
    }
}
