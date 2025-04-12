use super::{
    Cpu, Instruction, Register16,
    cycles::Cycles,
    instructions::{Address, Jump, jump},
};
use crate::emulation::MemoryBus;

impl Cpu {
    pub fn execute(&mut self, instruction: Instruction, memory_bus: &mut MemoryBus) -> Cycles {
        match instruction {
            Instruction::NoOperation => Cycles(1),
            Instruction::Jump(jump) => self.jump(jump),

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

            // Instruction::XorA(register) => {
            //     self.a = self.a ^ self.get_register8(register);
            //     self.flags.set(Flags::ZERO, self.a == 0);
            //     self.flags.remove(Flags::NEGATIVE);
            //     self.flags.remove(Flags::HALF_CARRY);
            //     self.flags.remove(Flags::CARRY);
            // }
            Instruction::Invalid(_) => panic!("Invalid instruction {}", instruction),
            _ => todo!("Implement instruction {}", instruction),
        }
    }

    fn jump(&mut self, jump: Jump) -> Cycles {
        match jump {
            Jump::Jump(condition, location) => {
                if let Some(condition) = condition {
                    todo!()
                }

                match location {
                    jump::Location::Address(address) => match address {
                        Address::Fixed(address) => {
                            self.program_counter = address;
                            Cycles(4)
                        }

                        Address::Relative(offset) => {
                            self.program_counter = match offset {
                                0.. => self.program_counter + offset.abs() as u16,
                                ..0 => self.program_counter - offset.abs() as u16,
                            };
                            Cycles(3)
                        }

                        _ => unreachable!(),
                    },

                    jump::Location::RegisterHl => {
                        self.program_counter = self.get_register16(Register16::Hl);
                        Cycles(1)
                    }
                }
            }
            Jump::Call(condition, location) => todo!(),
            Jump::Return(condition) => todo!(),
            Jump::ReturnAndEnableInterrupts => todo!(),
            Jump::Restart(_) => todo!(),
        }
    }
}
