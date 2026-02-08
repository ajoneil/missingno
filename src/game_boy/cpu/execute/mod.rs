use crate::game_boy::{
    MemoryMapped,
    memory::{MappedAddress, MemoryWrite},
};

use super::{
    Cpu, Instruction, Register16,
    cycles::Cycles,
    flags::Flags,
    instructions::{Address, Source8, Source16, Target8, Target16},
};

mod arithemetic;
mod bit_flag;
mod bit_shift;
mod bitwise;
mod carry_flag;
mod interrupt;
mod jump;
mod load;
mod stack;

pub struct OpResult(pub Cycles, pub Option<MemoryWrite>);
impl OpResult {
    pub fn cycles(cycles: u32) -> Self {
        OpResult(Cycles(cycles), None)
    }

    pub fn write8(address: u16, value: u8, cycles: Cycles) -> Self {
        OpResult(
            cycles,
            Some(MemoryWrite::Write8(MappedAddress::map(address), value)),
        )
    }

    pub fn write16(address: u16, value: u16, cycles: Cycles) -> Self {
        let high: u8 = (value >> 8) as u8;
        let low: u8 = (value & 0xff) as u8;

        OpResult(
            cycles,
            Some(MemoryWrite::Write16(
                (MappedAddress::map(address), low),
                (MappedAddress::map(address + 1), high),
            )),
        )
    }

    pub fn add_cycles(self, cycles: Cycles) -> Self {
        Self(self.0 + cycles, self.1)
    }
}

impl Cpu {
    pub fn execute(&mut self, instruction: Instruction, memory: &MemoryMapped) -> OpResult {
        match instruction {
            Instruction::Load(instruction) => self.execute_load(instruction, memory),
            Instruction::Arithmetic(instruction) => self.execute_arithmetic(instruction, memory),
            Instruction::Bitwise(instruction) => self.execute_bitwise(instruction, memory),
            Instruction::BitFlag(instruction) => self.execute_bit_flag(instruction, memory),
            Instruction::BitShift(instruction) => self.execute_bit_shift(instruction, memory),
            Instruction::Jump(instruction) => self.execute_jump(instruction, memory),
            Instruction::CarryFlag(instruction) => self.execute_carry_flag(instruction),
            Instruction::Stack(instruction) => self.execute_stack(instruction, memory),
            Instruction::Interrupt(instruction) => self.execute_interrupt(instruction),

            Instruction::DecimalAdjustAccumulator => {
                let value = if self.flags.contains(Flags::NEGATIVE) {
                    let mut adjustment = 0;
                    if self.flags.contains(Flags::HALF_CARRY) {
                        adjustment += 0x6;
                    }
                    if self.flags.contains(Flags::CARRY) {
                        adjustment += 0x60;
                    }

                    self.a.wrapping_sub(adjustment)
                } else {
                    let mut adjustment = 0;
                    if self.flags.contains(Flags::HALF_CARRY) || self.a & 0xf > 0x9 {
                        adjustment += 0x6;
                    }
                    if self.flags.contains(Flags::CARRY) || self.a > 0x99 {
                        adjustment += 0x60;
                        self.flags.insert(Flags::CARRY);
                    }

                    self.a.wrapping_add(adjustment)
                };

                self.flags.set(Flags::ZERO, value == 0);
                self.flags.remove(Flags::HALF_CARRY);
                self.a = value;

                OpResult::cycles(1)
            }

            Instruction::NoOperation => OpResult::cycles(1),
            Instruction::Stop => {
                self.halted = true;
                OpResult::cycles(0)
            }
            Instruction::Invalid(_) => panic!("Invalid instruction {}", instruction),
        }
    }

    fn fetch8(&mut self, source: Source8, memory: &MemoryMapped) -> (u8, Cycles) {
        match source {
            Source8::Constant(value) => (value, Cycles(1)),
            Source8::Register(register) => (self.get_register8(register), Cycles(0)),
            Source8::Memory(address) => match address {
                Address::Fixed(address) => (memory.read(address), Cycles(3)),
                Address::Relative(_) => unreachable!(),
                Address::High(offset) => (memory.read(0xff00 + offset as u16), Cycles(2)),
                Address::HighPlusC => (memory.read(0xff00 + self.c as u16), Cycles(1)),

                Address::Dereference(register) => {
                    let address = self.get_register16(register);
                    let value = memory.read(address);
                    (value, Cycles(1))
                }

                Address::DereferenceHlAndIncrement => {
                    let address = self.get_register16(Register16::Hl);
                    let value = memory.read(address);
                    self.set_register16(Register16::Hl, address + 1);
                    (value, Cycles(1))
                }

                Address::DereferenceHlAndDecrement => {
                    let address = self.get_register16(Register16::Hl);
                    let value = memory.read(address);
                    self.set_register16(Register16::Hl, address - 1);
                    (value, Cycles(1))
                }
            },
        }
    }

    fn set8(&mut self, target: Target8, value: u8) -> OpResult {
        match target {
            Target8::Register(register) => {
                self.set_register8(register, value);
                OpResult::cycles(0)
            }
            Target8::Memory(address) => match address {
                Address::Fixed(address) => OpResult::write8(address, value, Cycles(3)),
                Address::Relative(_) => unreachable!(),
                Address::High(offset) => OpResult::write8(0xff00 + offset as u16, value, Cycles(2)),
                Address::HighPlusC => OpResult::write8(0xff00 + self.c as u16, value, Cycles(1)),

                Address::Dereference(register) => {
                    let address = self.get_register16(register);
                    OpResult::write8(address, value, Cycles(1))
                }

                Address::DereferenceHlAndIncrement => {
                    let address = self.get_register16(Register16::Hl);
                    self.set_register16(Register16::Hl, address + 1);
                    OpResult::write8(address, value, Cycles(1))
                }

                Address::DereferenceHlAndDecrement => {
                    let address = self.get_register16(Register16::Hl);
                    self.set_register16(Register16::Hl, address - 1);
                    OpResult::write8(address, value, Cycles(1))
                }
            },
        }
    }

    fn fetch16(&mut self, source: Source16) -> (u16, Cycles) {
        match source {
            Source16::Constant(value) => (value, Cycles(2)),
            Source16::Register(register) => (self.get_register16(register), Cycles(1)),
            Source16::StackPointerWithOffset(offset) => {
                let sp = self.stack_pointer;
                let offset_u8 = offset as u8;
                let result = sp.wrapping_add(offset as i16 as u16);

                self.flags.remove(Flags::ZERO);
                self.flags.remove(Flags::NEGATIVE);
                self.flags.set(
                    Flags::HALF_CARRY,
                    (sp & 0xf) + (offset_u8 as u16 & 0xf) > 0xf,
                );
                self.flags
                    .set(Flags::CARRY, (sp & 0xff) + (offset_u8 as u16 & 0xff) > 0xff);

                (result, Cycles(2))
            }
        }
    }

    fn set16(&mut self, target: Target16, value: u16) -> OpResult {
        match target {
            Target16::Register(register) => {
                self.set_register16(register, value);
                OpResult::cycles(0)
            }
            Target16::Memory(address) => match address {
                Address::Fixed(address) => OpResult::write16(address, value, Cycles(2)),
                _ => unreachable!(),
            },
        }
    }
}
