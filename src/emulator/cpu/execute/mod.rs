use crate::emulator::{
    MemoryMapped,
    memory::{MappedAddress, MemoryWrite},
};

use super::{
    Cpu, Instruction, Register16,
    cycles::Cycles,
    instructions::{Address, Source8, Source16, Target8, Target16},
};

mod arithemetic;
mod bitwise;
mod interrupt;
mod jump;
mod load;

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
            Instruction::BitFlag(_) => todo!(),
            Instruction::BitShift(_) => todo!(),
            Instruction::Jump(instruction) => self.execute_jump(instruction),
            Instruction::CarryFlag(_) => todo!(),
            Instruction::StackPointer(_) => todo!(),
            Instruction::Interrupt(instruction) => self.execute_interrupt(instruction),
            Instruction::DecimalAdjustAccumulator => todo!(),
            Instruction::NoOperation => OpResult::cycles(1),
            Instruction::Stop => todo!(),
            Instruction::Invalid(_) => panic!("Invalid instruction {}", instruction),
        }
    }

    fn fetch8(&mut self, source: Source8, memory: &MemoryMapped) -> (u8, Cycles) {
        match source {
            Source8::Constant(value) => (value, Cycles(1)),
            Source8::Register(register) => (self.get_register8(register), Cycles(0)),
            Source8::Memory(address) => match address {
                Address::Fixed(_) => todo!(),
                Address::Relative(_) => todo!(),
                Address::High(offset) => (memory.read(0xff00 + offset as u16), Cycles(2)),
                Address::HighPlusC => todo!(),

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
                Address::Relative(_) => todo!(),
                Address::High(offset) => OpResult::write8(0xff00 + offset as u16, value, Cycles(2)),
                Address::HighPlusC => todo!(),

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

    fn fetch16(&self, source: Source16) -> (u16, Cycles) {
        match source {
            Source16::Constant(value) => (value, Cycles(2)),
            Source16::Register(register) => (self.get_register16(register), Cycles(1)),
            Source16::StackPointerWithOffset(offset) => (
                match offset {
                    0.. => self.stack_pointer + offset.abs() as u16,
                    ..0 => self.stack_pointer - offset.abs() as u16,
                },
                Cycles(2),
            ),
        }
    }

    fn set16(&mut self, target: Target16, value: u16) -> OpResult {
        match target {
            Target16::Register(register) => {
                self.set_register16(register, value);
                OpResult::cycles(0)
            }
            Target16::Memory(_) => todo!(),
        }
    }
}
