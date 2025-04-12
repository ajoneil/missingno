use super::{
    Cpu, Instruction,
    cycles::Cycles,
    instructions::{Address, Register16, Source8, Source16, Target8, Target16},
};
use crate::emulation::MemoryBus;

mod arithemetic;
mod bitwise;
mod jump;
mod load;

impl Cpu {
    pub fn execute(&mut self, instruction: Instruction, memory_bus: &mut MemoryBus) -> Cycles {
        match instruction {
            Instruction::Load(instruction) => self.execute_load(instruction, memory_bus),
            Instruction::Arithmetic(instruction) => self.execute_arithmetic(instruction),
            Instruction::Bitwise(instruction) => self.execute_bitwise(instruction, memory_bus),
            Instruction::BitFlag(bit_flag) => todo!(),
            Instruction::BitShift(bit_shift) => todo!(),
            Instruction::Jump(instruction) => self.execute_jump(instruction),
            Instruction::CarryFlag(carry_flag) => todo!(),
            Instruction::StackPointer(stack_pointer) => todo!(),
            Instruction::Interrupt(interrupt) => todo!(),
            Instruction::DecimalAdjustAccumulator => todo!(),
            Instruction::NoOperation => Cycles(1),
            Instruction::Stop => todo!(),
            Instruction::Invalid(_) => panic!("Invalid instruction {}", instruction),
            // Instruction::Decrement8(register) => {
            //     let value = self.get_register8(register);
            //     let new_value = if value == 0 { 0xff } else { value - 1 };
            //     self.set_register8(register, new_value);

            //     self.flags.set(Flags::ZERO, new_value == 0);
            //     self.flags.insert(Flags::NEGATIVE);

            //     // The half carry flag is set if we carry from bit 4 to 3
            //     // i.e. xxx10000 - 1 = xxx01111
            //     self.flags.set(Flags::HALF_CARRY, new_value & 0xf == 0xf);
            // }
        }
    }

    fn fetch8(&mut self, source: Source8, memory_bus: &MemoryBus) -> (u8, Cycles) {
        match source {
            Source8::Constant(value) => (value, Cycles(1)),
            Source8::Register(register) => (self.get_register8(register), Cycles(0)),
            Source8::Memory(address) => match address {
                Address::Fixed(_) => todo!(),
                Address::Relative(_) => todo!(),
                Address::Hram(_) => todo!(),
                Address::HramPlusC => todo!(),

                Address::Dereference(register) => {
                    let address = self.get_register16(register);
                    let value = memory_bus.read(address);
                    (value, Cycles(1))
                }

                Address::DereferenceHlAndIncrement => {
                    let address = self.get_register16(Register16::Hl);
                    let value = memory_bus.read(address);
                    self.set_register16(Register16::Hl, address + 1);
                    (value, Cycles(1))
                }

                Address::DereferenceHlAndDecrement => {
                    let address = self.get_register16(Register16::Hl);
                    let value = memory_bus.read(address);
                    self.set_register16(Register16::Hl, address - 1);
                    (value, Cycles(1))
                }

                Address::DereferenceFixed(_) => todo!(),
            },
        }
    }

    fn set8(&mut self, target: Target8, value: u8, memory_bus: &mut MemoryBus) -> Cycles {
        match target {
            Target8::Register(register) => {
                self.set_register8(register, value);
                Cycles(0)
            }
            Target8::Memory(address) => match address {
                Address::Fixed(_) => todo!(),
                Address::Relative(_) => todo!(),
                Address::Hram(_) => todo!(),
                Address::HramPlusC => todo!(),

                Address::Dereference(register) => {
                    let address = self.get_register16(register);
                    memory_bus.write(address, value);
                    Cycles(1)
                }

                Address::DereferenceHlAndIncrement => {
                    let address = self.get_register16(Register16::Hl);
                    memory_bus.write(address, value);
                    self.set_register16(Register16::Hl, address + 1);
                    Cycles(1)
                }

                Address::DereferenceHlAndDecrement => {
                    let address = self.get_register16(Register16::Hl);
                    memory_bus.write(address, value);
                    self.set_register16(Register16::Hl, address - 1);
                    Cycles(1)
                }

                Address::DereferenceFixed(_) => todo!(),
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

    fn set16(&mut self, target: Target16, value: u16) -> Cycles {
        match target {
            Target16::Register(register) => {
                self.set_register16(register, value);
                Cycles(0)
            }
            Target16::Memory(address) => todo!(),
        }
    }
}
