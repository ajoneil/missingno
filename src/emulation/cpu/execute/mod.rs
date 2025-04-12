use super::{
    Cpu, Instruction,
    cycles::Cycles,
    instructions::{Address, Source8},
};
use crate::emulation::MemoryBus;

mod arithemetic;
mod bitwise;
mod jump;

impl Cpu {
    pub fn execute(&mut self, instruction: Instruction, memory_bus: &mut MemoryBus) -> Cycles {
        match instruction {
            Instruction::Load(load) => todo!(),
            Instruction::Arithmetic(arithmetic) => self.execute_arithmetic(arithmetic),
            Instruction::Bitwise(bitwise) => self.execute_bitwise(bitwise, memory_bus),
            Instruction::BitFlag(bit_flag) => todo!(),
            Instruction::BitShift(bit_shift) => todo!(),
            Instruction::Jump(jump) => self.execute_jump(jump),
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

            // Instruction::Load8(destination, source) => {
            //     let value = match source {
            //         Load8Source::Constant(value) => value,
            //         Load8Source::Register(register) => self.get_register8(register),
            //     };

            //     match destination {
            //         Load8Target::Register(register) => self.set_register8(register, value),
            //         Load8Target::Pointer(pointer) => match pointer {
            //             Pointer::HlIncrement => {
            //                 let hl = self.get_register16(Register16::Hl);
            //                 memory_bus.write(hl, value);
            //                 self.set_register16(Register16::Hl, hl + 1);
            //             }
            //             Pointer::HlDecrement => {
            //                 let hl = self.get_register16(Register16::Hl);
            //                 memory_bus.write(hl, value);
            //                 self.set_register16(Register16::Hl, hl - 1);
            //             }
            //         },
            //     };
            // }

            // Instruction::Load16(destination, source) => {
            //     let value = match source {
            //         Load16Source::Constant(value) => value,
            //     };

            //     match destination {
            //         Load16Target::Register(register) => self.set_register16(register, value),
            //         Load16Target::StackPointer => self.stack_pointer = value,
            //     }
            // }
        }
    }

    fn fetch8(&self, source: Source8, memory_bus: &MemoryBus) -> (u8, Cycles) {
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
                Address::DereferenceHlAndIncrement => todo!(),
                Address::DereferenceHlAndDecrement => todo!(),
                Address::DereferenceFixed(_) => todo!(),
            },
        }
    }
}
