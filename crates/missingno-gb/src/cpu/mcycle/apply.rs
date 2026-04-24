use super::super::{
    Cpu, EiDelay, HaltState, InterruptMasterEnable,
    commit::Commit,
    flags::Flags,
    instructions::bit_shift::{Carry, Direction},
    instructions::{CarryFlag, Interrupt as InterruptInstruction},
    registers::Register16,
};
use super::{AluOp, PopAction, ReadAction, RmwOp};

impl Cpu {
    /// Route a `Commit` variant to the corresponding mutation. Mirrors the
    /// inline mutations currently in `build_*` for each opcode class. Used
    /// by `Cpu::commit` at retire edges — step 6 moves the inline code out
    /// of `build_*` so this becomes the single mutation site for every
    /// retiring instruction.
    pub(super) fn apply_commit(cpu: &mut Cpu, commit: Commit) {
        match commit {
            Commit::NoOperation => {}
            Commit::Invalid => {
                cpu.halt_state = HaltState::Halting;
            }

            Commit::LoadR8 { reg, value } => cpu.set_register8(reg, value),
            Commit::LoadR16 { reg, value } => cpu.set_register16(reg, value),

            Commit::IncR8 { reg } => {
                let val = cpu.get_register8(reg);
                let result = val.wrapping_add(1);
                cpu.flags.set(Flags::ZERO, result == 0);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.set(Flags::HALF_CARRY, result & 0b1111 == 0);
                cpu.set_register8(reg, result);
            }
            Commit::DecR8 { reg } => {
                let val = cpu.get_register8(reg);
                let result = val.wrapping_sub(1);
                cpu.flags.set(Flags::ZERO, result == 0);
                cpu.flags.insert(Flags::NEGATIVE);
                cpu.flags.set(Flags::HALF_CARRY, result & 0b1111 == 0b1111);
                cpu.set_register8(reg, result);
            }
            Commit::AluA { op, value } => Self::apply_alu(cpu, &op, value),

            Commit::Inc16 { reg } => {
                let val = cpu.get_register16(reg);
                cpu.set_register16(reg, val.wrapping_add(1));
            }
            Commit::Dec16 { reg } => {
                let val = cpu.get_register16(reg);
                cpu.set_register16(reg, val.wrapping_sub(1));
            }
            Commit::AddHl { source } => {
                let value = cpu.get_register16(source);
                let hl = cpu.get_register16(Register16::Hl);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.set(
                    Flags::HALF_CARRY,
                    (hl & 0xfff) as u32 + (value & 0xfff) as u32 > 0xfff,
                );
                cpu.flags
                    .set(Flags::CARRY, hl as u32 + value as u32 > 0xffff);
                cpu.set_register16(Register16::Hl, hl.wrapping_add(value));
            }
            Commit::AddSpOffset { offset } => {
                let sp = cpu.stack_pointer;
                let offset_u8 = offset as u8;
                cpu.stack_pointer = sp.wrapping_add(offset as i16 as u16);
                cpu.flags.remove(Flags::ZERO);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.set(
                    Flags::HALF_CARRY,
                    (sp & 0xf) + (offset_u8 as u16 & 0xf) > 0xf,
                );
                cpu.flags
                    .set(Flags::CARRY, (sp & 0xff) + (offset_u8 as u16 & 0xff) > 0xff);
            }
            Commit::LdHlSpOffset { offset } => {
                let sp = cpu.stack_pointer;
                let offset_u8 = offset as u8;
                let result = sp.wrapping_add(offset as i16 as u16);
                cpu.flags.remove(Flags::ZERO);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.set(
                    Flags::HALF_CARRY,
                    (sp & 0xf) + (offset_u8 as u16 & 0xf) > 0xf,
                );
                cpu.flags
                    .set(Flags::CARRY, (sp & 0xff) + (offset_u8 as u16 & 0xff) > 0xff);
                cpu.set_register16(Register16::Hl, result);
            }

            Commit::Daa => Self::apply_daa(cpu),
            Commit::CarryFlag(cf) => Self::apply_carry_flag(cpu, &cf),
            Commit::ComplementA => {
                cpu.a = !cpu.a;
                cpu.flags.insert(Flags::NEGATIVE);
                cpu.flags.insert(Flags::HALF_CARRY);
            }

            Commit::RotateAccumulator { direction, carry } => {
                let (new_value, new_carry) = Self::rotate(cpu, cpu.a, &direction, &carry);
                cpu.flags = if new_carry {
                    Flags::CARRY
                } else {
                    Flags::empty()
                };
                cpu.a = new_value;
            }
            Commit::RotateReg { reg, direction, carry } => {
                let val = cpu.get_register8(reg);
                let (new_value, new_carry) = Self::rotate(cpu, val, &direction, &carry);
                cpu.flags.set(Flags::ZERO, new_value == 0);
                cpu.flags.set(Flags::CARRY, new_carry);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.remove(Flags::HALF_CARRY);
                cpu.set_register8(reg, new_value);
            }
            Commit::ShiftArithmetical { reg, direction } => {
                let val = cpu.get_register8(reg);
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
                cpu.set_register8(reg, new_value);
            }
            Commit::ShiftRightLogical { reg } => {
                let val = cpu.get_register8(reg);
                let new_value = val >> 1;
                cpu.flags.set(Flags::CARRY, val & 0b0000_0001 != 0);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.remove(Flags::HALF_CARRY);
                cpu.flags.set(Flags::ZERO, new_value == 0);
                cpu.set_register8(reg, new_value);
            }
            Commit::SwapReg { reg } => {
                let val = cpu.get_register8(reg);
                let new_value = val << 4 | (val >> 4 & 0xf);
                cpu.flags = if new_value == 0 {
                    Flags::ZERO
                } else {
                    Flags::empty()
                };
                cpu.set_register8(reg, new_value);
            }
            Commit::BitTest { bit, reg } => {
                let val = cpu.get_register8(reg);
                cpu.flags.set(Flags::ZERO, val & (1 << bit) == 0);
                cpu.flags.remove(Flags::NEGATIVE);
                cpu.flags.insert(Flags::HALF_CARRY);
            }
            Commit::BitSet { bit, reg } => {
                let val = cpu.get_register8(reg);
                cpu.set_register8(reg, val | (1 << bit));
            }
            Commit::BitReset { bit, reg } => {
                let val = cpu.get_register8(reg);
                cpu.set_register8(reg, val & !(1 << bit));
            }

            Commit::JumpAbsolute { target } => {
                cpu.bus_counter = target;
                cpu.pc = target;
            }
            Commit::JumpReturnEnableInterrupts { target } => {
                cpu.ime.write_immediate(InterruptMasterEnable::Enabled);
                cpu.bus_counter = target;
                cpu.pc = target;
            }

            Commit::DisableInterrupts => {
                cpu.ime.write_immediate(InterruptMasterEnable::Disabled);
                cpu.ei_delay = None;
            }
            Commit::EnableInterrupts => {
                if cpu.ime.output() != InterruptMasterEnable::Enabled
                    && cpu.ei_delay.is_none()
                {
                    cpu.ei_delay = Some(EiDelay::Pending);
                }
            }

            Commit::EnterHalt | Commit::EnterStop => {
                cpu.halt_state = HaltState::Halting;
            }
        }
    }

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
                // On hardware, EI sets an RS latch — re-setting an already-set
                // latch is a no-op. Guard against overwriting Fired with Pending,
                // which would delay IME enablement by an extra instruction.
                if cpu.ime.output() != InterruptMasterEnable::Enabled
                    && cpu.ei_delay.is_none()
                {
                    cpu.ei_delay = Some(EiDelay::Pending);
                }
            }
            InterruptInstruction::Disable => {
                cpu.ime.write_immediate(InterruptMasterEnable::Disabled);
                // On real hardware, DI's combinational gate (g91) clears the
                // EI pipeline latch (g92). Without this, an EI;DI sequence
                // would let EI's delay survive and re-enable IME.
                cpu.ei_delay = None;
            }
            InterruptInstruction::Await => {
                cpu.halt_state = HaltState::Halted;
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
                cpu.bus_counter = value;
                cpu.pc = cpu.bus_counter;
            }
            PopAction::SetPcEnableInterrupts => {
                cpu.ime.write_immediate(InterruptMasterEnable::Enabled);
                cpu.bus_counter = value;
                cpu.pc = cpu.bus_counter;
            }
        }
    }
}
