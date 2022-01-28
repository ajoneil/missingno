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
        let instruction = mapper.read_pc(&mut self.pc);

        match instruction {
            // 8-bit load
            0x40 => Cycles(4), // ld b,b
            0x41 => ld_r_r(&mut self.b, self.c),
            0x42 => ld_r_r(&mut self.b, self.d),
            0x43 => ld_r_r(&mut self.b, self.e),
            0x44 => ld_r_r(&mut self.b, self.h),
            0x45 => ld_r_r(&mut self.b, self.l),
            0x47 => ld_r_r(&mut self.b, self.a),
            0x48 => ld_r_r(&mut self.c, self.b),
            0x49 => Cycles(4), // ld c,c
            0x4a => ld_r_r(&mut self.c, self.d),
            0x4b => ld_r_r(&mut self.c, self.e),
            0x4c => ld_r_r(&mut self.c, self.h),
            0x4d => ld_r_r(&mut self.c, self.l),
            0x4f => ld_r_r(&mut self.c, self.a),
            0x50 => ld_r_r(&mut self.d, self.b),
            0x51 => ld_r_r(&mut self.d, self.c),
            0x52 => Cycles(4), // ld d,d
            0x53 => ld_r_r(&mut self.d, self.e),
            0x54 => ld_r_r(&mut self.d, self.h),
            0x55 => ld_r_r(&mut self.d, self.l),
            0x57 => ld_r_r(&mut self.d, self.a),
            0x58 => ld_r_r(&mut self.e, self.b),
            0x59 => ld_r_r(&mut self.e, self.c),
            0x5a => ld_r_r(&mut self.e, self.d),
            0x5b => Cycles(4), // ld e,e
            0x5c => ld_r_r(&mut self.e, self.h),
            0x5d => ld_r_r(&mut self.e, self.l),
            0x5f => ld_r_r(&mut self.e, self.a),
            0x60 => ld_r_r(&mut self.h, self.b),
            0x61 => ld_r_r(&mut self.h, self.c),
            0x62 => ld_r_r(&mut self.h, self.d),
            0x63 => ld_r_r(&mut self.h, self.e),
            0x64 => Cycles(4), // ld h,h
            0x65 => ld_r_r(&mut self.h, self.l),
            0x67 => ld_r_r(&mut self.h, self.a),
            0x68 => ld_r_r(&mut self.l, self.b),
            0x69 => ld_r_r(&mut self.l, self.c),
            0x6a => ld_r_r(&mut self.l, self.d),
            0x6b => ld_r_r(&mut self.l, self.e),
            0x6c => ld_r_r(&mut self.l, self.h),
            0x6d => Cycles(4), // ld l,l
            0x6f => ld_r_r(&mut self.l, self.a),
            0x78 => ld_r_r(&mut self.a, self.b),
            0x79 => ld_r_r(&mut self.a, self.c),
            0x7a => ld_r_r(&mut self.a, self.d),
            0x7b => ld_r_r(&mut self.a, self.e),
            0x7c => ld_r_r(&mut self.a, self.h),
            0x7d => ld_r_r(&mut self.a, self.l),
            0x7f => Cycles(4), // ld a,a
            0x06 => ld_r_n(&mut self.b, mapper.read_pc(&mut self.pc)),
            0x0e => ld_r_n(&mut self.c, mapper.read_pc(&mut self.pc)),
            0x16 => ld_r_n(&mut self.d, mapper.read_pc(&mut self.pc)),
            0x1e => ld_r_n(&mut self.e, mapper.read_pc(&mut self.pc)),
            0x26 => ld_r_n(&mut self.h, mapper.read_pc(&mut self.pc)),
            0x2e => ld_r_n(&mut self.l, mapper.read_pc(&mut self.pc)),
            0x3e => ld_r_n(&mut self.a, mapper.read_pc(&mut self.pc)),
            0x46 => ld_r_rrptr(&mut self.b, self.h, self.l, mapper),
            0x4e => ld_r_rrptr(&mut self.c, self.h, self.l, mapper),
            0x56 => ld_r_rrptr(&mut self.d, self.h, self.l, mapper),
            0x5e => ld_r_rrptr(&mut self.e, self.h, self.l, mapper),
            0x66 => {
                let h = self.h;
                ld_r_rrptr(&mut self.h, h, self.l, mapper)
            }
            0x6e => {
                let l = self.l;
                ld_r_rrptr(&mut self.l, self.h, l, mapper)
            }
            0x7e => ld_r_rrptr(&mut self.a, self.h, self.l, mapper),
            0x70 => ld_hlptr_r(self.h, self.l, self.b, mapper),
            0x71 => ld_hlptr_r(self.h, self.l, self.c, mapper),
            0x72 => ld_hlptr_r(self.h, self.l, self.d, mapper),
            0x73 => ld_hlptr_r(self.h, self.l, self.e, mapper),
            0x74 => ld_hlptr_r(self.h, self.l, self.h, mapper),
            0x75 => ld_hlptr_r(self.h, self.l, self.l, mapper),
            0x77 => ld_hlptr_r(self.h, self.l, self.a, mapper),
            0x36 => ld_hlptr_n(self.h, self.l, mapper.read_pc(&mut self.pc), mapper),
            0x0a => ld_r_rrptr(&mut self.a, self.b, self.c, mapper),
            0x1a => ld_r_rrptr(&mut self.a, self.d, self.e, mapper),
            0xf0 => ld_a_nhptr(&mut self.a, mapper.read_pc(&mut self.pc), mapper),
            0xe0 => ld_nhptr_a(mapper.read_pc(&mut self.pc), self.a, mapper),
            0xf2 => ld_a_chptr(&mut self.a, self.c, mapper),
            0xe2 => ld_chptr_a(self.c, self.a, mapper),
            0x32 => ld_hlptr_dec_a(&mut self.h, &mut self.l, self.a, mapper),

            // 16-bit load
            0x01 => ld_rr_nn(&mut self.b, &mut self.c, mapper.read_word_pc(&mut self.pc)),
            0x11 => ld_rr_nn(&mut self.d, &mut self.e, mapper.read_word_pc(&mut self.pc)),
            0x21 => ld_rr_nn(&mut self.h, &mut self.l, mapper.read_word_pc(&mut self.pc)),
            0x31 => ld_sp_nn(&mut self.sp, mapper.read_word_pc(&mut self.pc)),

            // 8-bit arithmetic and logic
            0xa8 => xor_r(self.b, &mut self.a, &mut self.f),
            0xa9 => xor_r(self.c, &mut self.a, &mut self.f),
            0xaa => xor_r(self.d, &mut self.a, &mut self.f),
            0xab => xor_r(self.e, &mut self.a, &mut self.f),
            0xac => xor_r(self.h, &mut self.a, &mut self.f),
            0xad => xor_r(self.l, &mut self.a, &mut self.f),
            0xaf => xor_r(self.a, &mut self.a, &mut self.f),
            0xb8 => cp_r(self.b, self.a, &mut self.f),
            0xb9 => cp_r(self.c, self.a, &mut self.f),
            0xba => cp_r(self.d, self.a, &mut self.f),
            0xbb => cp_r(self.e, self.a, &mut self.f),
            0xbc => cp_r(self.h, self.a, &mut self.f),
            0xbd => cp_r(self.b, self.a, &mut self.f),
            0xbf => cp_r(self.a, self.a, &mut self.f),
            0xfe => cp_n(mapper.read_pc(&mut self.pc), self.a, &mut self.f),
            0xbe => cp_hlptr(self.h, self.l, self.a, &mut self.f, mapper),
            0x05 => dec_r(&mut self.b, &mut self.f),
            0x0d => dec_r(&mut self.c, &mut self.f),
            0x15 => dec_r(&mut self.d, &mut self.f),
            0x1d => dec_r(&mut self.e, &mut self.f),
            0x25 => dec_r(&mut self.h, &mut self.f),
            0x2d => dec_r(&mut self.l, &mut self.f),
            0x3d => dec_r(&mut self.a, &mut self.f),

            // 16-bit arithmetic and logic

            // rotate and shift

            // cpu control
            0x00 => nop(),
            0xf3 => di(&mut self.ime),
            0xfb => ei(&mut self.ime),

            // jump
            0xc3 => {
                let nn = mapper.read_word_pc(&mut self.pc);
                jp_nn(&mut self.pc, nn)
            }

            0x18 => {
                let distance = mapper.read_pc(&mut self.pc);
                jr(&mut self.pc, distance)
            }
            0x20 => {
                let distance = mapper.read_pc(&mut self.pc);
                jr_if(&mut self.pc, distance, !self.f.contains(Flags::Z))
            }
            0x28 => {
                let distance = mapper.read_pc(&mut self.pc);
                jr_if(&mut self.pc, distance, self.f.contains(Flags::Z))
            }
            0x30 => {
                let distance = mapper.read_pc(&mut self.pc);
                jr_if(&mut self.pc, distance, !self.f.contains(Flags::C))
            }
            0x38 => {
                let distance = mapper.read_pc(&mut self.pc);
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
            _ => panic!(
                "Unimplemented instruction {:x} at {:x}",
                instruction, self.pc
            ),
        }
    }

    // fn read_hl(&self, mmu: &Mmu, video: &Video) -> u8 {
    //     mmu.read((self.h as u16 * 256) + self.l as u16, video)
    // }

    // fn increment_hl(&mut self) {
    //     if self.l == 0xff {
    //         self.h = self.h + 1;
    //         self.l = 0x00;
    //     } else {
    //         self.l = self.l + 1;
    //     }
    // }
}

impl fmt::Debug for Cpu {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "pc: {:04x} sp: {:04x} af: {:02x}{:02x} bc: {:02x}{:02x} de: {:02x}{:02x} hl: {:02x}{:02x} flags: {:?}",
               self.pc, self.sp, self.a, self.f, self.b, self.c, self.d, self.e, self.h, self.l, self.f)
    }
}
