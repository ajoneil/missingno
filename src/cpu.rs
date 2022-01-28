use crate::mmu::Mapper;
use crate::mmu::Mmu;
use crate::ops::*;
use crate::video::Video;
use bitflags::bitflags;
use std::fmt;

pub struct Cpu {
    a: u8,
    f: Flags,
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    h: u8,
    l: u8,
    sp: u16,
    pc: u16,
    ime: bool,
}

pub struct Cycles(pub u8);

bitflags! {
    pub struct Flags: u8 {
        const Z = 0b10000000;
        const N = 0b01000000;
        const H = 0b00100000;
        const C = 0b00010000;
    }
}

impl Cpu {
    pub fn new() -> Cpu {
        Cpu {
            a: 0x01,
            f: Flags::empty(),
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xd8,
            h: 0x01,
            l: 0x4d,
            sp: 0xfffe,
            pc: 0x100,
            ime: false,
        }
    }

    pub fn step(&mut self, mmu: &mut Mmu, video: &mut Video) -> Cycles {
        let mapper = &mut Mapper::new(mmu, video);

        let instruction = Cpu::read(&mut self.pc, mapper);
        match instruction {
            // 8-bit load
            0x06 => ld_r_n(&mut self.b, Cpu::read(&mut self.pc, mapper)),
            0x0e => ld_r_n(&mut self.c, Cpu::read(&mut self.pc, mapper)),
            0x16 => ld_r_n(&mut self.d, Cpu::read(&mut self.pc, mapper)),
            0x1e => ld_r_n(&mut self.e, Cpu::read(&mut self.pc, mapper)),
            0x26 => ld_r_n(&mut self.h, Cpu::read(&mut self.pc, mapper)),
            0x2e => ld_r_n(&mut self.l, Cpu::read(&mut self.pc, mapper)),
            0x3e => ld_r_n(&mut self.a, Cpu::read(&mut self.pc, mapper)),

            0x01 => ld_rr_nn(
                &mut self.b,
                &mut self.c,
                Cpu::read_word(&mut self.pc, mapper),
            ),
            0x11 => ld_rr_nn(
                &mut self.d,
                &mut self.e,
                Cpu::read_word(&mut self.pc, mapper),
            ),
            0x21 => ld_rr_nn(
                &mut self.h,
                &mut self.l,
                Cpu::read_word(&mut self.pc, mapper),
            ),

            0x32 => ld_hlptr_dec_a(&mut self.h, &mut self.l, self.a, mapper),

            // 16-bit load
            0x31 => ld_sp_nn(&mut self.sp, Cpu::read_word(&mut self.pc, mapper)),

            // 8-bit arithmetic and logic
            0x05 => dec_r(&mut self.b, &mut self.f),
            0x0d => dec_r(&mut self.c, &mut self.f),
            0x15 => dec_r(&mut self.d, &mut self.f),
            0x1d => dec_r(&mut self.e, &mut self.f),
            0x25 => dec_r(&mut self.h, &mut self.f),
            0x2d => dec_r(&mut self.l, &mut self.f),
            0x3d => dec_r(&mut self.a, &mut self.f),

            0xa8 => xor_r(self.b, &mut self.a, &mut self.f),
            0xa9 => xor_r(self.c, &mut self.a, &mut self.f),
            0xaa => xor_r(self.d, &mut self.a, &mut self.f),
            0xab => xor_r(self.e, &mut self.a, &mut self.f),
            0xac => xor_r(self.h, &mut self.a, &mut self.f),
            0xad => xor_r(self.l, &mut self.a, &mut self.f),
            0xaf => xor_r(self.a, &mut self.a, &mut self.f),

            // 16-bit arithmetic and logic

            // rotate and shift

            // cpu control
            0x00 => nop(),
            0xf3 => di(&mut self.ime),
            0xfb => ei(&mut self.ime),

            // jump
            0xc3 => {
                let nn = Cpu::read_word(&mut self.pc, &mapper);
                jp_nn(&mut self.pc, nn)
            }

            0x18 => {
                let distance = Cpu::read(&mut self.pc, &mapper);
                jr(&mut self.pc, distance)
            }
            0x20 => {
                let distance = Cpu::read(&mut self.pc, &mapper);
                jr_if(&mut self.pc, distance, !self.f.contains(Flags::Z))
            }
            0x28 => {
                let distance = Cpu::read(&mut self.pc, &mapper);
                jr_if(&mut self.pc, distance, self.f.contains(Flags::Z))
            }
            0x30 => {
                let distance = Cpu::read(&mut self.pc, &mapper);
                jr_if(&mut self.pc, distance, !self.f.contains(Flags::C))
            }
            0x38 => {
                let distance = Cpu::read(&mut self.pc, &mapper);
                jr_if(&mut self.pc, distance, self.f.contains(Flags::C))
            }

            // 0x22 => {
            //     self.write_hl(mmu, self.a, video);
            //     self.increment_hl();
            //     8
            // } // ldi (hl),a
            // 0x2a => {
            //     self.a = self.read_hl(mmu, video);
            //     self.increment_hl();
            //     8
            // } // ldi a,(hl)
            // 0x36 => {
            //     let val = Cpu::read_and_inc_pc(&mut self.pc, mmu, video);
            //     self.write_hl(mmu, val, video);
            //     12
            // } // ld (hl),n
            // 0x3a => {
            //     self.a = self.read_hl(mmu, video);
            //     self.decrement_hl();
            //     8
            // } // ldd a,(hl)
            // 0x40 => 4, // ld b,b
            // 0x41 => {
            //     self.b = self.c;
            //     4
            // } // ld b,c
            // 0x42 => {
            //     self.b = self.d;
            //     4
            // } // ld b,d
            // 0x43 => {
            //     self.b = self.e;
            //     4
            // } // ld b,e
            // 0x44 => {
            //     self.b = self.h;
            //     4
            // } // ld b,h
            // 0x45 => {
            //     self.b = self.l;
            //     4
            // } // ld b,l
            // 0x47 => {
            //     self.b = self.a;
            //     4
            // } // ld b,a
            // 0x48 => {
            //     self.c = self.b;
            //     4
            // } // ld c,b
            // 0x49 => 4, // ld c,c
            // 0x4a => {
            //     self.c = self.d;
            //     4
            // } // ld c,d
            // 0x4b => {
            //     self.c = self.e;
            //     4
            // } // ld c,e
            // 0x4c => {
            //     self.c = self.h;
            //     4
            // } // ld c,h
            // 0x4d => {
            //     self.c = self.l;
            //     4
            // } // ld c,l
            // 0x4f => {
            //     self.c = self.a;
            //     4
            // } // ld c,a
            // 0x50 => {
            //     self.d = self.b;
            //     4
            // } // ld d,b
            // 0x51 => {
            //     self.d = self.c;
            //     4
            // } // ld d,c
            // 0x52 => 4, // ld d,d
            // 0x53 => {
            //     self.d = self.e;
            //     4
            // } // ld d,e
            // 0x54 => {
            //     self.d = self.h;
            //     4
            // } // ld d,h
            // 0x55 => {
            //     self.d = self.l;
            //     4
            // } // ld d,l
            // 0x57 => {
            //     self.d = self.a;
            //     4
            // } // ld d,a
            // 0x58 => {
            //     self.e = self.b;
            //     4
            // } // ld e,b
            // 0x59 => {
            //     self.e = self.c;
            //     4
            // } // ld e,c
            // 0x5a => {
            //     self.e = self.d;
            //     4
            // } // ld e,d
            // 0x5b => 4, // ld e,e
            // 0x5c => {
            //     self.e = self.h;
            //     4
            // } // ld e,h
            // 0x5d => {
            //     self.e = self.l;
            //     4
            // } // ld e,l
            // 0x5f => {
            //     self.e = self.a;
            //     4
            // } // ld e,a
            // 0x60 => {
            //     self.h = self.b;
            //     4
            // } // ld h,b
            // 0x61 => {
            //     self.h = self.c;
            //     4
            // } // ld h,c
            // 0x62 => {
            //     self.h = self.d;
            //     4
            // } // ld h,d
            // 0x63 => {
            //     self.h = self.e;
            //     4
            // } // ld h,e
            // 0x64 => 4, // ld h,h
            // 0x65 => {
            //     self.h = self.l;
            //     4
            // } // ld h,l
            // 0x67 => {
            //     self.h = self.a;
            //     4
            // } // ld h,a
            // 0x68 => {
            //     self.l = self.b;
            //     4
            // } // ld l,b
            // 0x69 => {
            //     self.l = self.c;
            //     4
            // } // ld l,c
            // 0x6a => {
            //     self.l = self.d;
            //     4
            // } // ld l,d
            // 0x6b => {
            //     self.l = self.e;
            //     4
            // } // ld l,e
            // 0x6c => {
            //     self.l = self.h;
            //     4
            // } // ld l,h
            // 0x6d => 4, // ld l,l
            // 0x6f => {
            //     self.l = self.a;
            //     4
            // } // ld l,a
            // 0x78 => {
            //     self.a = self.b;
            //     4
            // } // ld a,b
            // 0x79 => {
            //     self.a = self.c;
            //     4
            // } // ld a,c
            // 0x7a => {
            //     self.a = self.d;
            //     4
            // } // ld a,d
            // 0x7b => {
            //     self.a = self.e;
            //     4
            // } // ld a,e
            // 0x7c => {
            //     self.a = self.h;
            //     4
            // } // ld a,h
            // 0x7d => {
            //     self.a = self.l;
            //     4
            // } // ld a,l
            // 0x7f => 4, // ld a,a
            // 0x98 => {
            //     let result: i16 =
            //         self.a as i16 - self.b as i16 - (if self.carry() { 1 } else { 0 });
            //     self.set_carry(result < 0);
            //     self.set_z(result == 0);
            //     self.a = (0xff & result) as u8;
            //     4
            // } // sbc a,b
            // 0x99 => {
            //     let result: i16 =
            //         self.a as i16 - self.c as i16 - (if self.carry() { 1 } else { 0 });
            //     self.set_carry(result < 0);
            //     self.set_z(result == 0);
            //     self.a = (0xff & result) as u8;
            //     4
            // } // sbc a,c
            // 0x9a => {
            //     let result: i16 =
            //         self.a as i16 - self.d as i16 - (if self.carry() { 1 } else { 0 });
            //     self.set_carry(result < 0);
            //     self.set_z(result == 0);
            //     self.a = (0xff & result) as u8;
            //     4
            // } // sbc a,d
            // 0x9b => {
            //     let result: i16 =
            //         self.a as i16 - self.e as i16 - (if self.carry() { 1 } else { 0 });
            //     self.set_carry(result < 0);
            //     self.set_z(result == 0);
            //     self.a = (0xff & result) as u8;
            //     4
            // } // sbc a,e
            // 0x9c => {
            //     let result: i16 =
            //         self.a as i16 - self.h as i16 - (if self.carry() { 1 } else { 0 });
            //     self.set_carry(result < 0);
            //     self.set_z(result == 0);
            //     self.a = (0xff & result) as u8;
            //     4
            // } // sbc a,h
            // 0x9d => {
            //     let result: i16 =
            //         self.a as i16 - self.l as i16 - (if self.carry() { 1 } else { 0 });
            //     self.set_carry(result < 0);
            //     self.set_z(result == 0);
            //     self.a = (0xff & result) as u8;
            //     4
            // } // sbc a,l
            // 0x9e => {
            //     let result: i16 = self.a as i16
            //         - self.read_hl(mmu, video) as i16
            //         - (if self.carry() { 1 } else { 0 });
            //     self.set_carry(result < 0);
            //     self.set_z(result == 0);
            //     self.a = (0xff & result) as u8;
            //     8
            // } // sbc a,(hl)
            // 0x9f => {
            //     let result: i16 = if self.carry() { -1 } else { 0 };
            //     self.set_carry(result < 0);
            //     self.set_z(result == 0);
            //     self.a = (0xff & result) as u8;
            //     4
            // } // sbc a,a
            // 0xae => {
            //     self.a = self.a ^ self.read_hl(mmu, video);
            //     let z = self.a == 0;
            //     self.set_z(z);
            //     self.set_carry(false);
            //     8
            // } // xor (hl)
            // 0xb8 => {
            //     let z = self.a == self.b;
            //     self.set_z(z);
            //     let carry = self.a < self.b;
            //     self.set_carry(carry);
            //     4
            // } // cp b
            // 0xb9 => {
            //     let z = self.a == self.c;
            //     self.set_z(z);
            //     let carry = self.a < self.c;
            //     self.set_carry(carry);
            //     4
            // } // cp c
            // 0xba => {
            //     let z = self.a == self.d;
            //     self.set_z(z);
            //     let carry = self.a < self.d;
            //     self.set_carry(carry);
            //     4
            // } // cp d
            // 0xbb => {
            //     let z = self.a == self.e;
            //     self.set_z(z);
            //     let carry = self.a < self.e;
            //     self.set_carry(carry);
            //     4
            // } // cp e
            // 0xbc => {
            //     let z = self.a == self.h;
            //     self.set_z(z);
            //     let carry = self.a < self.h;
            //     self.set_carry(carry);
            //     4
            // } // cp h
            // 0xbd => {
            //     let z = self.a == self.l;
            //     self.set_z(z);
            //     let carry = self.a < self.l;
            //     self.set_carry(carry);
            //     4
            // } // cp l
            // 0xbe => {
            //     let val = self.read_hl(mmu, video);
            //     let z = self.a == val;
            //     self.set_z(z);
            //     let carry = self.a < val;
            //     self.set_carry(carry);
            //     8
            // } // cp (hl)
            // 0xbf => {
            //     self.set_z(true);
            //     self.set_carry(false);
            //     4
            // } // cp a
            // 0xc4 => {
            //     let address = Cpu::read_word_and_inc_pc(&mut self.pc, mmu, video);
            //     if !self.z() {
            //         self.sp = self.sp - 2;
            //         mmu.write_word(self.sp, self.pc, video);
            //         self.pc = address;
            //     }
            //     12
            // } // call nz,nn
            // 0xcc => {
            //     let address = Cpu::read_word_and_inc_pc(&mut self.pc, mmu, video);
            //     if self.z() {
            //         self.sp = self.sp - 2;
            //         mmu.write_word(self.sp, self.pc, video);
            //         self.pc = address;
            //     }
            //     12
            // } // call z,nn
            // 0xd4 => {
            //     let address = Cpu::read_word_and_inc_pc(&mut self.pc, mmu, video);
            //     if !self.carry() {
            //         self.sp = self.sp - 2;
            //         mmu.write_word(self.sp, self.pc, video);
            //         self.pc = address;
            //     }
            //     12
            // } // call nc,nn
            // 0xdc => {
            //     let address = Cpu::read_word_and_inc_pc(&mut self.pc, mmu, video);
            //     if self.carry() {
            //         self.sp = self.sp - 2;
            //         mmu.write_word(self.sp, self.pc, video);
            //         self.pc = address;
            //     }
            //     12
            // } // call c,nn
            // 0xe0 => {
            //     let address = 0xff00 + Cpu::read_and_inc_pc(&mut self.pc, mmu, video) as u16;
            //     mmu.write(address, self.a, video);
            //     12
            // } // ldh (n),a
            // 0xea => {
            //     let address = Cpu::read_word_and_inc_pc(&mut self.pc, mmu, video);
            //     mmu.write(address, self.a, video);
            //     16
            // } // ld (nn),a
            // 0xee => {
            //     self.a = self.a ^ Cpu::read_and_inc_pc(&mut self.pc, mmu, video);
            //     let z = self.a == 0;
            //     self.set_z(z);
            //     self.set_carry(false);
            //     8
            // } // xor nn
            // 0xf0 => {
            //     let address = 0xff00 + Cpu::read_and_inc_pc(&mut self.pc, mmu, video) as u16;
            //     self.a = mmu.read(address, video);
            //     12
            // } // ldh a,(n)
            // 0xfe => {
            //     let val = Cpu::read_and_inc_pc(&mut self.pc, mmu, video);
            //     let z = self.a == val;
            //     self.set_z(z);
            //     let carry = self.a < val;
            //     self.set_carry(carry);
            //     8
            // } // cp n
            _ => panic!(
                "Unimplemented instruction {:x} at {:x}",
                instruction, self.pc
            ),
        }
    }

    fn read(pc: &mut u16, mapper: &Mapper) -> u8 {
        let byte = mapper.read(*pc);
        *pc += 1;
        byte
    }

    fn read_word(pc: &mut u16, mapper: &Mapper) -> u16 {
        let byte = mapper.read_word(*pc);
        *pc += 2;
        byte
    }

    fn read_hl(&self, mmu: &Mmu, video: &Video) -> u8 {
        mmu.read((self.h as u16 * 256) + self.l as u16, video)
    }

    fn increment_hl(&mut self) {
        if self.l == 0xff {
            self.h = self.h + 1;
            self.l = 0x00;
        } else {
            self.l = self.l + 1;
        }
    }
}

impl fmt::Debug for Cpu {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "pc: {:04x} sp: {:04x} af: {:02x}{:02x} bc: {:02x}{:02x} de: {:02x}{:02x} hl: {:02x}{:02x} flags: {:?}",
               self.pc, self.sp, self.a, self.f, self.b, self.c, self.d, self.e, self.h, self.l, self.f)
    }
}
