mod cycles;
mod flags;
mod instructions;

use super::MemoryBus;
pub use flags::{Flag, Flags};
pub use instructions::{Instruction, Register8, Register16};

pub struct Cpu {
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,

    pub stack_pointer: u16,
    pub program_counter: u16,

    pub flags: Flags,

    pub interrupt_master_enable: bool,
    pub halted: bool,
}

struct ProgramCounterIterator<'a> {
    pc: &'a mut u16,
    memory_bus: &'a MemoryBus,
}

impl<'a> Iterator for ProgramCounterIterator<'a> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.memory_bus.read(*self.pc);
        *self.pc += 1;
        Some(value)
    }
}

impl Cpu {
    pub fn new(checksum: u8) -> Cpu {
        Cpu {
            a: 0x01,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xd8,
            h: 0x01,
            l: 0x4d,

            stack_pointer: 0xfffe,
            program_counter: 0x0100,

            flags: if checksum == 0 {
                Flags::ZERO
            } else {
                Flags::ZERO | Flags::CARRY | Flags::HALF_CARRY
            },

            interrupt_master_enable: false,
            halted: false,
        }
    }

    pub fn step(&mut self, memory_bus: &mut MemoryBus) {
        let mut pc_iterator = ProgramCounterIterator {
            pc: &mut self.program_counter,
            memory_bus,
        };
        let instruction = Instruction::decode(&mut pc_iterator).unwrap();
        self.execute(instruction, memory_bus);
    }

    fn execute(&mut self, instruction: Instruction, memory_bus: &mut MemoryBus) {
        match instruction {
            Instruction::NoOperation => {}

            // Instruction::Jump(address) => match address {
            //     JumpAddress::Absolute(address) => self.program_counter = address,
            // },

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

    fn get_register8(&self, register: Register8) -> u8 {
        match register {
            Register8::A => self.a,
            Register8::B => self.b,
            Register8::C => self.c,
            Register8::D => self.d,
            Register8::E => self.e,
            Register8::H => self.h,
            Register8::L => self.l,
        }
    }

    fn set_register8(&mut self, register: Register8, value: u8) {
        match register {
            Register8::A => self.a = value,
            Register8::B => self.b = value,
            Register8::C => self.c = value,
            Register8::D => self.d = value,
            Register8::E => self.e = value,
            Register8::H => self.h = value,
            Register8::L => self.l = value,
        }
    }

    fn get_register16(&self, register: Register16) -> u16 {
        match register {
            Register16::Bc => u16::from_le_bytes([self.b, self.c]),
            Register16::De => u16::from_le_bytes([self.d, self.e]),
            Register16::Hl => u16::from_le_bytes([self.h, self.l]),
            Register16::StackPointer => self.stack_pointer,
            Register16::Af => u16::from_le_bytes([self.a, self.flags.bits()]),
        }
    }

    fn set_register16(&mut self, register: Register16, value: u16) {
        let high = (value / 0x100) as u8;
        let low = (value % 0x100) as u8;

        match register {
            Register16::Bc => {
                self.b = high;
                self.c = low;
            }
            Register16::De => {
                self.d = high;
                self.c = low;
            }
            Register16::Hl => {
                self.h = high;
                self.l = low;
            }
            Register16::StackPointer => self.stack_pointer = value,
            Register16::Af => {
                self.a = high;
                self.flags = Flags::from_bits_retain(low);
            }
        }
    }
}
