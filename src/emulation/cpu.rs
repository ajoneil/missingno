use crate::emulation::{
    Instruction, MemoryBus,
    instructions::{JumpAddress, Load8Source, Load8Target, Load16Source, Load16Target},
};
use bitflags::bitflags;
use std::fmt::{self, Display};

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

#[derive(Clone, Copy)]
pub enum Register8 {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
}

pub enum Register16 {
    Bc,
    De,
    Hl,
    StackPointer,
}

pub enum Pointer {
    HlIncrement,
    HlDecrement,
}

impl Display for Register8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::A => "a",
                Self::B => "b",
                Self::C => "c",
                Self::D => "d",
                Self::E => "e",
                Self::H => "h",
                Self::L => "l",
            }
        )
    }
}

impl Display for Register16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Bc => "bc",
                Self::De => "de",
                Self::Hl => "hl",
                Self::StackPointer => "sp",
            }
        )
    }
}

impl Display for Pointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}]",
            match self {
                Self::HlIncrement => "hl+",
                Self::HlDecrement => "hl-",
            }
        )
    }
}

#[derive(Clone, Copy, PartialEq, PartialOrd)]
pub struct Cycles(pub u32);

impl std::ops::Add for Cycles {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Cycles {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl std::ops::AddAssign for Cycles {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs
    }
}

impl std::ops::SubAssign for Cycles {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs
    }
}

bitflags! {
    #[derive(Copy,Clone,Debug)]
    pub struct Flags: u8 {
        const ZERO = 0b10000000;
        const NEGATIVE = 0b01000000;
        const HALF_CARRY = 0b00100000;
        const CARRY = 0b00010000;

        const _OTHER = !0;
    }
}

bitflags! {
    #[derive(Copy,Clone,Debug)]
    pub struct Interrupts: u8 {
        const JOYPAD = 0b00010000;
        const SERIAL = 0b00001000;
        const TIMER  = 0b00000100;
        const LCD    = 0b00000010;
        const VBLANK = 0b00000001;

        const _OTHER = !0;
    }
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
        let instruction = Instruction::decode(&mut pc_iterator);
        self.execute(instruction, memory_bus);
    }

    fn execute(&mut self, instruction: Instruction, memory_bus: &mut MemoryBus) {
        match instruction {
            Instruction::NoOperation => {}

            Instruction::Jump(address) => match address {
                JumpAddress::Absolute(address) => self.program_counter = address,
            },

            Instruction::Decrement8(register) => {
                let value = self.get_register8(register);
                let new_value = if value == 0 { 0xff } else { value - 1 };
                self.set_register8(register, new_value);

                self.flags.set(Flags::ZERO, new_value == 0);
                self.flags.insert(Flags::NEGATIVE);

                // The half carry flag is set if we carry from bit 4 to 3
                // i.e. xxx10000 - 1 = xxx01111
                self.flags.set(Flags::HALF_CARRY, new_value & 0xf == 0xf);
            }

            Instruction::Load8(destination, source) => {
                let value = match source {
                    Load8Source::Constant(value) => value,
                    Load8Source::Register(register) => self.get_register8(register),
                };

                match destination {
                    Load8Target::Register(register) => self.set_register8(register, value),
                    Load8Target::Pointer(pointer) => match pointer {
                        Pointer::HlIncrement => {
                            let hl = self.get_register16(Register16::Hl);
                            memory_bus.write(hl, value);
                            self.set_register16(Register16::Hl, hl + 1);
                        }
                        Pointer::HlDecrement => {
                            let hl = self.get_register16(Register16::Hl);
                            memory_bus.write(hl, value);
                            self.set_register16(Register16::Hl, hl - 1);
                        }
                    },
                };
            }

            Instruction::Load16(destination, source) => {
                let value = match source {
                    Load16Source::Constant(value) => value,
                };

                match destination {
                    Load16Target::Register(register) => self.set_register16(register, value),
                }
            }

            Instruction::XorA(register) => {
                self.a = self.a ^ self.get_register8(register);
                self.flags.set(Flags::ZERO, self.a == 0);
                self.flags.remove(Flags::NEGATIVE);
                self.flags.remove(Flags::HALF_CARRY);
                self.flags.remove(Flags::CARRY);
            }

            Instruction::Unknown(_) => panic!("Unimplemented instruction {}", instruction),
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
            Register16::Bc => self.b as u16 * 0x100 + self.c as u16,
            Register16::De => self.d as u16 * 0x100 + self.d as u16,
            Register16::Hl => self.h as u16 * 0x100 + self.l as u16,
            Register16::StackPointer => self.stack_pointer,
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
        }
    }

    // pub fn step(
    //     &mut self,
    //     mmu: &mut Mmu,
    //     video: &mut Video,
    //     timers: &mut Timers,
    //     joypad: &mut Joypad,
    // ) -> Cycles {
    //     if self.ime || self.halted {
    //         let interrupts = mmu.interrupt_flags().intersection(mmu.enabled_interrupts());
    //         if !interrupts.is_empty() {
    //             if self.halted {
    //                 println!("resuming..");
    //                 self.halted = false;
    //             }

    //             if self.ime {
    //                 self.ime = false;
    //                 self.sp -= 2;
    //                 mmu.write_word(self.sp, self.pc, video, timers, joypad);

    //                 if interrupts.contains(Interrupts::VBLANK) {
    //                     self.pc = 0x40;
    //                     mmu.reset_interrupt_flag(Interrupts::VBLANK);
    //                     println!("vblank interrupt!");
    //                 } else if interrupts.contains(Interrupts::SERIAL) {
    //                     self.pc = 0x58;
    //                     mmu.reset_interrupt_flag(Interrupts::SERIAL);
    //                     println!("serial interrupt!");
    //                 } else {
    //                     panic!("unhandled interrupt {:?}", interrupts)
    //                 }

    //                 return Cycles(20);
    //             }
    //         }
    //     }

    //     if self.halted {
    //         return Cycles(4);
    //     }

    //     let mapper = &mut Mapper::new(mmu, video, timers, joypad);
    //     let instruction = mapper.read_pc(&mut self.pc);
    //     match instruction {
    //         // 8-bit load
    //         0x40 => Cycles(4), // ld b,b
    //         0x41 => ld_r_r(&mut self.b, self.c),
    //         0x42 => ld_r_r(&mut self.b, self.d),
    //         0x43 => ld_r_r(&mut self.b, self.e),
    //         0x44 => ld_r_r(&mut self.b, self.h),
    //         0x45 => ld_r_r(&mut self.b, self.l),
    //         0x47 => ld_r_r(&mut self.b, self.a),
    //         0x48 => ld_r_r(&mut self.c, self.b),
    //         0x49 => Cycles(4), // ld c,c
    //         0x4a => ld_r_r(&mut self.c, self.d),
    //         0x4b => ld_r_r(&mut self.c, self.e),
    //         0x4c => ld_r_r(&mut self.c, self.h),
    //         0x4d => ld_r_r(&mut self.c, self.l),
    //         0x4f => ld_r_r(&mut self.c, self.a),
    //         0x50 => ld_r_r(&mut self.d, self.b),
    //         0x51 => ld_r_r(&mut self.d, self.c),
    //         0x52 => Cycles(4), // ld d,d
    //         0x53 => ld_r_r(&mut self.d, self.e),
    //         0x54 => ld_r_r(&mut self.d, self.h),
    //         0x55 => ld_r_r(&mut self.d, self.l),
    //         0x57 => ld_r_r(&mut self.d, self.a),
    //         0x58 => ld_r_r(&mut self.e, self.b),
    //         0x59 => ld_r_r(&mut self.e, self.c),
    //         0x5a => ld_r_r(&mut self.e, self.d),
    //         0x5b => Cycles(4), // ld e,e
    //         0x5c => ld_r_r(&mut self.e, self.h),
    //         0x5d => ld_r_r(&mut self.e, self.l),
    //         0x5f => ld_r_r(&mut self.e, self.a),
    //         0x60 => ld_r_r(&mut self.h, self.b),
    //         0x61 => ld_r_r(&mut self.h, self.c),
    //         0x62 => ld_r_r(&mut self.h, self.d),
    //         0x63 => ld_r_r(&mut self.h, self.e),
    //         0x64 => Cycles(4), // ld h,h
    //         0x65 => ld_r_r(&mut self.h, self.l),
    //         0x67 => ld_r_r(&mut self.h, self.a),
    //         0x68 => ld_r_r(&mut self.l, self.b),
    //         0x69 => ld_r_r(&mut self.l, self.c),
    //         0x6a => ld_r_r(&mut self.l, self.d),
    //         0x6b => ld_r_r(&mut self.l, self.e),
    //         0x6c => ld_r_r(&mut self.l, self.h),
    //         0x6d => Cycles(4), // ld l,l
    //         0x6f => ld_r_r(&mut self.l, self.a),
    //         0x78 => ld_r_r(&mut self.a, self.b),
    //         0x79 => ld_r_r(&mut self.a, self.c),
    //         0x7a => ld_r_r(&mut self.a, self.d),
    //         0x7b => ld_r_r(&mut self.a, self.e),
    //         0x7c => ld_r_r(&mut self.a, self.h),
    //         0x7d => ld_r_r(&mut self.a, self.l),
    //         0x7f => Cycles(4), // ld a,a
    //         0x46 => ld_r_rrptr(&mut self.b, self.h, self.l, mapper),
    //         0x4e => ld_r_rrptr(&mut self.c, self.h, self.l, mapper),
    //         0x56 => ld_r_rrptr(&mut self.d, self.h, self.l, mapper),
    //         0x5e => ld_r_rrptr(&mut self.e, self.h, self.l, mapper),
    //         0x66 => {
    //             let h = self.h;
    //             ld_r_rrptr(&mut self.h, h, self.l, mapper)
    //         }
    //         0x6e => {
    //             let l = self.l;
    //             ld_r_rrptr(&mut self.l, self.h, l, mapper)
    //         }
    //         0x7e => ld_r_rrptr(&mut self.a, self.h, self.l, mapper),
    //         0x70 => ld_hlptr_r(self.h, self.l, self.b, mapper),
    //         0x71 => ld_hlptr_r(self.h, self.l, self.c, mapper),
    //         0x72 => ld_hlptr_r(self.h, self.l, self.d, mapper),
    //         0x73 => ld_hlptr_r(self.h, self.l, self.e, mapper),
    //         0x74 => ld_hlptr_r(self.h, self.l, self.h, mapper),
    //         0x75 => ld_hlptr_r(self.h, self.l, self.l, mapper),
    //         0x77 => ld_hlptr_r(self.h, self.l, self.a, mapper),
    //         0x36 => ld_hlptr_n(self.h, self.l, mapper.read_pc(&mut self.pc), mapper),
    //         0x0a => ld_r_rrptr(&mut self.a, self.b, self.c, mapper),
    //         0x1a => ld_r_rrptr(&mut self.a, self.d, self.e, mapper),
    //         0xfa => ld_a_nnptr(&mut self.a, mapper.read_word_pc(&mut self.pc), mapper),
    //         0x02 => ld_rrptr_a(self.b, self.c, self.a, mapper),
    //         0x12 => ld_rrptr_a(self.d, self.e, self.a, mapper),
    //         0xea => ld_nnptr_a(mapper.read_word_pc(&mut self.pc), self.a, mapper),
    //         0xf0 => ld_a_nhptr(&mut self.a, mapper.read_pc(&mut self.pc), mapper),
    //         0xe0 => ld_nhptr_a(mapper.read_pc(&mut self.pc), self.a, mapper),
    //         0xf2 => ld_a_chptr(&mut self.a, self.c, mapper),
    //         0xe2 => ld_chptr_a(self.c, self.a, mapper),
    //         0x22 => ld_hlptr_inc_a(&mut self.h, &mut self.l, self.a, mapper),
    //         0x2a => ld_a_hlptr_inc(&mut self.a, &mut self.h, &mut self.l, mapper),
    //         0x32 => ld_hlptr_dec_a(&mut self.h, &mut self.l, self.a, mapper),
    //         0x3a => ld_a_hlptr_dec(&mut self.a, &mut self.h, &mut self.l, mapper),

    //         // 16-bit load
    //         0x08 => ld_nnptr_sp(mapper.read_word_pc(&mut self.pc), self.sp, mapper),
    //         0xf9 => ld_sp_hl(&mut self.sp, self.h, self.l),
    //         0xc5 => push_rr(self.b, self.c, &mut self.sp, mapper),
    //         0xd5 => push_rr(self.d, self.e, &mut self.sp, mapper),
    //         0xe5 => push_rr(self.h, self.l, &mut self.sp, mapper),
    //         0xf5 => push_rr(self.a, self.f.bits(), &mut self.sp, mapper),
    //         0xc1 => pop_rr(&mut self.b, &mut self.c, &mut self.sp, mapper),
    //         0xd1 => pop_rr(&mut self.d, &mut self.e, &mut self.sp, mapper),
    //         0xe1 => pop_rr(&mut self.h, &mut self.l, &mut self.sp, mapper),
    //         0xf1 => pop_af(&mut self.a, &mut self.f, &mut self.sp, mapper),

    //         // 8-bit arithmetic and logic
    //         0x80 => add_a_r(&mut self.a, self.b, &mut self.f),
    //         0x81 => add_a_r(&mut self.a, self.c, &mut self.f),
    //         0x82 => add_a_r(&mut self.a, self.d, &mut self.f),
    //         0x83 => add_a_r(&mut self.a, self.e, &mut self.f),
    //         0x84 => add_a_r(&mut self.a, self.h, &mut self.f),
    //         0x85 => add_a_r(&mut self.a, self.l, &mut self.f),
    //         0x87 => {
    //             let a = self.a;
    //             add_a_r(&mut self.a, a, &mut self.f)
    //         }
    //         0xc6 => add_a_n(&mut self.a, mapper.read_pc(&mut self.pc), &mut self.f),
    //         0x86 => add_a_hlptr(&mut self.a, self.h, self.l, &mut self.f, mapper),
    //         0x88 => adc_a_r(&mut self.a, self.b, &mut self.f),
    //         0x89 => adc_a_r(&mut self.a, self.c, &mut self.f),
    //         0x8a => adc_a_r(&mut self.a, self.d, &mut self.f),
    //         0x8b => adc_a_r(&mut self.a, self.e, &mut self.f),
    //         0x8c => adc_a_r(&mut self.a, self.h, &mut self.f),
    //         0x8d => adc_a_r(&mut self.a, self.l, &mut self.f),
    //         0x8f => {
    //             let a = self.a;
    //             adc_a_r(&mut self.a, a, &mut self.f)
    //         }
    //         0xce => adc_a_n(&mut self.a, mapper.read_pc(&mut self.pc), &mut self.f),
    //         0x8e => adc_a_hlptr(&mut self.a, self.h, self.l, &mut self.f, mapper),
    //         0x90 => sub_r(&mut self.a, self.b, &mut self.f),
    //         0x91 => sub_r(&mut self.a, self.c, &mut self.f),
    //         0x92 => sub_r(&mut self.a, self.d, &mut self.f),
    //         0x93 => sub_r(&mut self.a, self.e, &mut self.f),
    //         0x94 => sub_r(&mut self.a, self.h, &mut self.f),
    //         0x95 => sub_r(&mut self.a, self.l, &mut self.f),
    //         0x97 => {
    //             let a = self.a;
    //             sub_r(&mut self.a, a, &mut self.f)
    //         }
    //         0xd6 => sub_n(&mut self.a, mapper.read_pc(&mut self.pc), &mut self.f),
    //         0x96 => sub_hlptr(&mut self.a, self.h, self.l, &mut self.f, mapper),
    //         0x98 => sbc_a_r(&mut self.a, self.b, &mut self.f),
    //         0x99 => sbc_a_r(&mut self.a, self.c, &mut self.f),
    //         0x9a => sbc_a_r(&mut self.a, self.d, &mut self.f),
    //         0x9b => sbc_a_r(&mut self.a, self.e, &mut self.f),
    //         0x9c => sbc_a_r(&mut self.a, self.h, &mut self.f),
    //         0x9d => sbc_a_r(&mut self.a, self.l, &mut self.f),
    //         0x9f => {
    //             let a = self.a;
    //             sbc_a_r(&mut self.a, a, &mut self.f)
    //         }
    //         0xde => sbc_a_n(&mut self.a, mapper.read_pc(&mut self.pc), &mut self.f),
    //         0x9e => sbc_a_hlptr(&mut self.a, self.h, self.l, &mut self.f, mapper),
    //         0xa0 => and_r(&mut self.a, self.b, &mut self.f),
    //         0xa1 => and_r(&mut self.a, self.c, &mut self.f),
    //         0xa2 => and_r(&mut self.a, self.d, &mut self.f),
    //         0xa3 => and_r(&mut self.a, self.e, &mut self.f),
    //         0xa4 => and_r(&mut self.a, self.h, &mut self.f),
    //         0xa5 => and_r(&mut self.a, self.l, &mut self.f),
    //         0xa7 => {
    //             let a = self.a;
    //             and_r(&mut self.a, a, &mut self.f)
    //         }
    //         0xe6 => and_n(&mut self.a, mapper.read_pc(&mut self.pc), &mut self.f),
    //         0xa6 => and_hlptr(&mut self.a, self.h, self.l, &mut self.f, mapper),
    //         0xee => xor_n(&mut self.a, mapper.read_pc(&mut self.pc), &mut self.f),
    //         0xae => xor_hlptr(&mut self.a, self.h, self.l, &mut self.f, mapper),
    //         0xb0 => or_r(&mut self.a, self.b, &mut self.f),
    //         0xb1 => or_r(&mut self.a, self.c, &mut self.f),
    //         0xb2 => or_r(&mut self.a, self.d, &mut self.f),
    //         0xb3 => or_r(&mut self.a, self.e, &mut self.f),
    //         0xb4 => or_r(&mut self.a, self.h, &mut self.f),
    //         0xb5 => or_r(&mut self.a, self.l, &mut self.f),
    //         0xb7 => {
    //             let a = self.a;
    //             or_r(&mut self.a, a, &mut self.f)
    //         }
    //         0xf6 => or_n(&mut self.a, mapper.read_pc(&mut self.pc), &mut self.f),
    //         0xb6 => or_hlptr(&mut self.a, self.h, self.l, &mut self.f, mapper),
    //         0xb8 => cp_r(self.a, self.b, &mut self.f),
    //         0xb9 => cp_r(self.a, self.c, &mut self.f),
    //         0xba => cp_r(self.a, self.d, &mut self.f),
    //         0xbb => cp_r(self.a, self.e, &mut self.f),
    //         0xbc => cp_r(self.a, self.h, &mut self.f),
    //         0xbd => cp_r(self.a, self.b, &mut self.f),
    //         0xbf => cp_r(self.a, self.a, &mut self.f),
    //         0xfe => cp_n(self.a, mapper.read_pc(&mut self.pc), &mut self.f),
    //         0xbe => cp_hlptr(self.a, self.h, self.l, &mut self.f, mapper),
    //         0x04 => inc_r(&mut self.b, &mut self.f),
    //         0x0c => inc_r(&mut self.c, &mut self.f),
    //         0x14 => inc_r(&mut self.d, &mut self.f),
    //         0x1c => inc_r(&mut self.e, &mut self.f),
    //         0x24 => inc_r(&mut self.h, &mut self.f),
    //         0x2c => inc_r(&mut self.l, &mut self.f),
    //         0x3c => inc_r(&mut self.a, &mut self.f),
    //         0x34 => inc_hlptr(self.h, self.l, &mut self.f, mapper),
    //         0x05 => dec_r(&mut self.b, &mut self.f),
    //         0x0d => dec_r(&mut self.c, &mut self.f),
    //         0x15 => dec_r(&mut self.d, &mut self.f),
    //         0x1d => dec_r(&mut self.e, &mut self.f),
    //         0x25 => dec_r(&mut self.h, &mut self.f),
    //         0x2d => dec_r(&mut self.l, &mut self.f),
    //         0x3d => dec_r(&mut self.a, &mut self.f),
    //         0x35 => dec_hlptr(self.h, self.l, &mut self.f, mapper),
    //         0x27 => daa(&mut self.a, &mut self.f),
    //         0x2f => cpl(&mut self.a, &mut self.f),

    //         // 16-bit arithmetic and logic
    //         0x09 => add_hl_rr(&mut self.h, &mut self.l, self.b, self.c, &mut self.f),
    //         0x19 => add_hl_rr(&mut self.h, &mut self.l, self.d, self.e, &mut self.f),
    //         0x29 => {
    //             let (h, l) = (self.h, self.l);
    //             add_hl_rr(&mut self.h, &mut self.l, h, l, &mut self.f)
    //         }
    //         0x39 => add_hl_sp(&mut self.h, &mut self.l, self.sp, &mut self.f),
    //         0x03 => inc_rr(&mut self.b, &mut self.c),
    //         0x13 => inc_rr(&mut self.d, &mut self.e),
    //         0x23 => inc_rr(&mut self.h, &mut self.l),
    //         0x33 => inc_sp(&mut self.sp),
    //         0x0b => dec_rr(&mut self.b, &mut self.c),
    //         0x1b => dec_rr(&mut self.d, &mut self.e),
    //         0x2b => dec_rr(&mut self.h, &mut self.l),
    //         0x3b => dec_sp(&mut self.sp),
    //         0xe8 => add_sp_dd(
    //             &mut self.sp,
    //             mapper.read_pc(&mut self.pc) as i8,
    //             &mut self.f,
    //         ),
    //         0xf8 => ld_hl_sp_dd(
    //             &mut self.h,
    //             &mut self.l,
    //             self.sp,
    //             mapper.read_pc(&mut self.pc) as i8,
    //             &mut self.f,
    //         ),

    //         // rotate and shift
    //         0x07 => rlca(&mut self.a, &mut self.f),
    //         0x17 => rla(&mut self.a, &mut self.f),
    //         0x0f => rrca(&mut self.a, &mut self.f),
    //         0x1f => rra(&mut self.a, &mut self.f),

    //         // cpu control
    //         0x00 => nop(),
    //         0x76 => {
    //             println!("halted!");
    //             self.halted = true;
    //             Cycles(4)
    //         }
    //         0xf3 => di(&mut self.ime),
    //         0xfb => ei(&mut self.ime),

    //         // jump
    //         0xe9 => jp_hl(&mut self.pc, self.h, self.l),

    //         0xc2 => {
    //             let nn = mapper.read_word_pc(&mut self.pc);
    //             jp_f_nn(&mut self.pc, !self.f.contains(Flags::Z), nn)
    //         }
    //         0xca => {
    //             let nn = mapper.read_word_pc(&mut self.pc);
    //             jp_f_nn(&mut self.pc, self.f.contains(Flags::Z), nn)
    //         }
    //         0xd2 => {
    //             let nn = mapper.read_word_pc(&mut self.pc);
    //             jp_f_nn(&mut self.pc, !self.f.contains(Flags::C), nn)
    //         }
    //         0xda => {
    //             let nn = mapper.read_word_pc(&mut self.pc);
    //             jp_f_nn(&mut self.pc, self.f.contains(Flags::C), nn)
    //         }

    //         0x18 => {
    //             let distance = mapper.read_pc(&mut self.pc);
    //             jr(&mut self.pc, distance)
    //         }
    //         0x20 => {
    //             let distance = mapper.read_pc(&mut self.pc);
    //             jr_if(&mut self.pc, distance, !self.f.contains(Flags::Z))
    //         }
    //         0x28 => {
    //             let distance = mapper.read_pc(&mut self.pc);
    //             jr_if(&mut self.pc, distance, self.f.contains(Flags::Z))
    //         }
    //         0x30 => {
    //             let distance = mapper.read_pc(&mut self.pc);
    //             jr_if(&mut self.pc, distance, !self.f.contains(Flags::C))
    //         }
    //         0x38 => {
    //             let distance = mapper.read_pc(&mut self.pc);
    //             jr_if(&mut self.pc, distance, self.f.contains(Flags::C))
    //         }

    //         0xcd => {
    //             let nn = mapper.read_word_pc(&mut self.pc);
    //             call_nn(&mut self.pc, &mut self.sp, nn, mapper)
    //         }
    //         0xc4 => {
    //             let nn = mapper.read_word_pc(&mut self.pc);
    //             call_f_nn(
    //                 &mut self.pc,
    //                 &mut self.sp,
    //                 !self.f.contains(Flags::Z),
    //                 nn,
    //                 mapper,
    //             )
    //         }
    //         0xc9 => ret(&mut self.pc, &mut self.sp, mapper),
    //         0xc0 => ret_f(
    //             &mut self.pc,
    //             &mut self.sp,
    //             !self.f.contains(Flags::N),
    //             mapper,
    //         ),
    //         0xc8 => ret_f(
    //             &mut self.pc,
    //             &mut self.sp,
    //             self.f.contains(Flags::Z),
    //             mapper,
    //         ),
    //         0xd0 => ret_f(
    //             &mut self.pc,
    //             &mut self.sp,
    //             !self.f.contains(Flags::C),
    //             mapper,
    //         ),
    //         0xd8 => ret_f(
    //             &mut self.pc,
    //             &mut self.sp,
    //             self.f.contains(Flags::C),
    //             mapper,
    //         ),
    //         0xd9 => reti(&mut self.pc, &mut self.sp, &mut self.ime, mapper),
    //         0xc7 => rst_n(&mut self.pc, &mut self.sp, 0x00, mapper),
    //         0xcf => rst_n(&mut self.pc, &mut self.sp, 0x08, mapper),
    //         0xd7 => rst_n(&mut self.pc, &mut self.sp, 0x10, mapper),
    //         0xdf => rst_n(&mut self.pc, &mut self.sp, 0x18, mapper),
    //         0xe7 => rst_n(&mut self.pc, &mut self.sp, 0x20, mapper),
    //         0xef => rst_n(&mut self.pc, &mut self.sp, 0x28, mapper),
    //         0xf7 => rst_n(&mut self.pc, &mut self.sp, 0x30, mapper),
    //         0xff => rst_n(&mut self.pc, &mut self.sp, 0x38, mapper),

    //         0xcb => {
    //             let cb_instruction = mapper.read_pc(&mut self.pc);
    //             match cb_instruction {
    //                 0x30 => swap_r(&mut self.b),
    //                 0x31 => swap_r(&mut self.c),
    //                 0x32 => swap_r(&mut self.d),
    //                 0x33 => swap_r(&mut self.e),
    //                 0x34 => swap_r(&mut self.h),
    //                 0x35 => swap_r(&mut self.l),
    //                 0x37 => swap_r(&mut self.a),

    //                 0xcf => set_n_r(1, &mut self.a),

    //                 0x87 => res_n_r(0, &mut self.a),
    //                 _ => {
    //                     panic!(
    //                         "Unimplemented instruction cb{:2x} at {:4x}",
    //                         cb_instruction, self.pc
    //                     );
    //                 }
    //             }
    //         }

    //         _ => {
    //             panic!(
    //                 "Unimplemented instruction {:x} at {:x}",
    //                 instruction, self.pc
    //             );
    //         }
    //     }
    // }
}
