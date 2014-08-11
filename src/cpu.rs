use mmu::Mmu;

pub struct Cpu {
    a: u8,
    f: u8,
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    h: u8,
    l: u8,
    sp: u16,
    pc: u16
}

static Z_FLAG: u8 = 0b10000000;
static N_FLAG: u8 = 0b01000000;
static H_FLAG: u8 = 0b00100000;
static C_FLAG: u8 = 0b00010000;

impl Cpu {
    pub fn new() -> Cpu {
        Cpu {
            a: 0x01,
            f: 0xb0,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xd8,
            h: 0x01,
            l: 0x4d,
            sp: 0xfffe,
            pc: 0x100
        }
    }

    pub fn step(&mut self, mmu: &mut Mmu) -> int {
        let instruction = self.read_and_inc_pc(mmu);
        match instruction {
            0x00 => 4, // nop
            0x01 => { self.c = self.read_and_inc_pc(mmu); self.b = self.read_and_inc_pc(mmu); 12 } // ld bc,nn
            0x05 => { self.b = self.b - 1; let z = self.b == 0; self.set_z(z); 4 } // dec b
            0x06 => { self.b = self.read_and_inc_pc(mmu); 8 } // ld b,n
            0x0d => { self.c = self.c - 1; let z = self.c == 0; self.set_z(z); 4 } // dec c
            0x0e => { self.c = self.read_and_inc_pc(mmu); 8 } // ld c,n
            0x11 => { self.e = self.read_and_inc_pc(mmu); self.d = self.read_and_inc_pc(mmu); 12 } // ld de,nn
            0x15 => { self.d = self.d - 1; let z = self.d == 0; self.set_z(z); 4 } // dec d
            0x16 => { self.d = self.read_and_inc_pc(mmu); 8 } // ld d,n
            0x1d => { self.e = self.e - 1; let z = self.e == 0; self.set_z(z); 4 } // dec e
            0x1e => { self.e = self.read_and_inc_pc(mmu); 8 } // ld e,n
            0x20 => { let distance = self.read_and_inc_pc(mmu); if !self.z() { self.jr(distance); 12 } else { 8 } } // jr nz,n
            0x21 => { self.l = self.read_and_inc_pc(mmu); self.h = self.read_and_inc_pc(mmu); 12 } // ld hl,nn
            0x22 => { self.write_hl(mmu, self.a); self.increment_hl(); 8 } // ldi (hl),a
            0x25 => { self.h = self.h - 1; let z = self.h == 0; self.set_z(z); 4 } // dec h
            0x26 => { self.h = self.read_and_inc_pc(mmu); 8 } // ld h,n
            0x28 => { let distance = self.read_and_inc_pc(mmu); if self.z() { self.jr(distance); 12 } else { 8 } } // jr z,n
            0x2a => { self.a = self.read_hl(mmu); self.increment_hl(); 8 } // ldi a,(hl)
            0x2d => { self.l = self.l - 1; let z = self.l == 0; self.set_z(z); 4 } // dec l
            0x2e => { self.l = self.read_and_inc_pc(mmu); 8 } // ld l,n
            0x30 => { let distance = self.read_and_inc_pc(mmu); if !self.carry() { self.jr(distance); 12 } else { 8 } } // jr nc,n
            0x31 => { self.sp = self.read_word_and_inc_pc(mmu); 12 } // ld sp,nn
            0x32 => { self.write_hl(mmu, self.a); self.decrement_hl(); 8 } // ldd (hl),a
            0x38 => { let distance = self.read_and_inc_pc(mmu); if self.carry() { self.jr(distance); 12 } else { 8 } } // jr c,n
            0x3a => { self.a = self.read_hl(mmu); self.decrement_hl(); 8 } // ldd a,(hl)
            0x3d => { self.a = self.a - 1; let z = self.a == 0; self.set_z(z); 4 } // dec a
            0x3e => { self.a = self.read_and_inc_pc(mmu); 8 } // ld a,n
            0x40 => { 4 } // ld b,b
            0x41 => { self.b = self.c; 4 } // ld b,c
            0x42 => { self.b = self.d; 4 } // ld b,d
            0x43 => { self.b = self.e; 4 } // ld b,e
            0x44 => { self.b = self.h; 4 } // ld b,h
            0x45 => { self.b = self.l; 4 } // ld b,l
            0x47 => { self.b = self.a; 4 } // ld b,a
            0x48 => { self.c = self.b; 4 } // ld c,b
            0x49 => { 4 } // ld c,c
            0x4a => { self.c = self.d; 4 } // ld c,d
            0x4b => { self.c = self.e; 4 } // ld c,e
            0x4c => { self.c = self.h; 4 } // ld c,h
            0x4d => { self.c = self.l; 4 } // ld c,l
            0x4f => { self.c = self.a; 4 } // ld c,a
            0x50 => { self.d = self.b; 4 } // ld d,b
            0x51 => { self.d = self.c; 4 } // ld d,c
            0x52 => { 4 } // ld d,d
            0x53 => { self.d = self.e; 4 } // ld d,e
            0x54 => { self.d = self.h; 4 } // ld d,h
            0x55 => { self.d = self.l; 4 } // ld d,l
            0x57 => { self.d = self.a; 4 } // ld d,a
            0x58 => { self.e = self.b; 4 } // ld e,b
            0x59 => { self.e = self.c; 4 } // ld e,c
            0x5a => { self.e = self.d; 4 } // ld e,d
            0x5b => { 4 } // ld e,e
            0x5c => { self.e = self.h; 4 } // ld e,h
            0x5d => { self.e = self.l; 4 } // ld e,l
            0x5f => { self.e = self.a; 4 } // ld e,a
            0x60 => { self.h = self.b; 4 } // ld h,b
            0x61 => { self.h = self.c; 4 } // ld h,c
            0x62 => { self.h = self.d; 4 } // ld h,d
            0x63 => { self.h = self.e; 4 } // ld h,e
            0x64 => { 4 } // ld h,h
            0x65 => { self.h = self.l; 4 } // ld h,l
            0x67 => { self.h = self.a; 4 } // ld h,a
            0x68 => { self.l = self.b; 4 } // ld l,b
            0x69 => { self.l = self.c; 4 } // ld l,c
            0x6a => { self.l = self.d; 4 } // ld l,d
            0x6b => { self.l = self.e; 4 } // ld l,e
            0x6c => { self.l = self.h; 4 } // ld l,h
            0x6d => { 4 } // ld l,l
            0x6f => { self.l = self.a; 4 } // ld l,a
            0x78 => { self.a = self.b; 4 } // ld a,b
            0x79 => { self.a = self.c; 4 } // ld a,c
            0x7a => { self.a = self.d; 4 } // ld a,d
            0x7b => { self.a = self.e; 4 } // ld a,e
            0x7c => { self.a = self.h; 4 } // ld a,h
            0x7d => { self.a = self.l; 4 } // ld a,l
            0x7f => { 4 } // ld a,a
            0x98 => { let result : i16 = self.a as i16 - self.b as i16 - (if self.carry() { 1 } else { 0 } );
                      self.set_carry(result < 0); self.set_z(result == 0); self.a = (0xff & result) as u8; 4 } // sbc a,b
            0x99 => { let result : i16 = self.a as i16 - self.c as i16 - (if self.carry() { 1 } else { 0 } );
                      self.set_carry(result < 0); self.set_z(result == 0); self.a = (0xff & result) as u8; 4 } // sbc a,c
            0x9a => { let result : i16 = self.a as i16 - self.d as i16 - (if self.carry() { 1 } else { 0 } );
                      self.set_carry(result < 0); self.set_z(result == 0); self.a = (0xff & result) as u8; 4 } // sbc a,d
            0x9b => { let result : i16 = self.a as i16 - self.e as i16 - (if self.carry() { 1 } else { 0 } );
                      self.set_carry(result < 0); self.set_z(result == 0); self.a = (0xff & result) as u8; 4 } // sbc a,e
            0x9c => { let result : i16 = self.a as i16 - self.h as i16 - (if self.carry() { 1 } else { 0 } );
                      self.set_carry(result < 0); self.set_z(result == 0); self.a = (0xff & result) as u8; 4 } // sbc a,h
            0x9d => { let result : i16 = self.a as i16 - self.l as i16 - (if self.carry() { 1 } else { 0 } );
                      self.set_carry(result < 0); self.set_z(result == 0); self.a = (0xff & result) as u8; 4 } // sbc a,l
            0x9e => { let result : i16 = self.a as i16 - self.read_hl(mmu) as i16 - (if self.carry() { 1 } else { 0 } );
                      self.set_carry(result < 0); self.set_z(result == 0); self.a = (0xff & result) as u8; 8 } // sbc a,(hl)
            0x9f => { let result : i16 = if self.carry() { -1 } else { 0 };
                      self.set_carry(result < 0); self.set_z(result == 0); self.a = (0xff & result) as u8; 4 } // sbc a,a
            0xa8 => { self.a = self.a ^ self.b; let z = self.a == 0; self.set_z(z); self.set_carry(false); 4 } // xor b
            0xa9 => { self.a = self.a ^ self.c; let z = self.a == 0; self.set_z(z); self.set_carry(false); 4 } // xor c
            0xaa => { self.a = self.a ^ self.d; let z = self.a == 0; self.set_z(z); self.set_carry(false); 4 } // xor d
            0xab => { self.a = self.a ^ self.e; let z = self.a == 0; self.set_z(z); self.set_carry(false); 4 } // xor e
            0xac => { self.a = self.a ^ self.h; let z = self.a == 0; self.set_z(z); self.set_carry(false); 4 } // xor h
            0xad => { self.a = self.a ^ self.l; let z = self.a == 0; self.set_z(z); self.set_carry(false); 4 } // xor l
            0xae => { self.a = self.a ^ self.read_hl(mmu); let z = self.a == 0; self.set_z(z); self.set_carry(false); 8 } // xor (hl)
            0xaf => { self.a = self.a ^ self.a; let z = self.a == 0; self.set_z(z); self.set_carry(false); 4 } // xor a
            0xb8 => { let z = self.a == self.b; self.set_z(z); let carry = self.a < self.b; self.set_carry(carry); 4} // cp b
            0xb9 => { let z = self.a == self.c; self.set_z(z); let carry = self.a < self.c; self.set_carry(carry); 4} // cp c
            0xba => { let z = self.a == self.d; self.set_z(z); let carry = self.a < self.d; self.set_carry(carry); 4} // cp d
            0xbb => { let z = self.a == self.e; self.set_z(z); let carry = self.a < self.e; self.set_carry(carry); 4} // cp e
            0xbc => { let z = self.a == self.h; self.set_z(z); let carry = self.a < self.h; self.set_carry(carry); 4} // cp h
            0xbd => { let z = self.a == self.l; self.set_z(z); let carry = self.a < self.l; self.set_carry(carry); 4} // cp l
            0xbe => { let val = self.read_hl(mmu); let z = self.a == val; self.set_z(z); let carry = self.a < val; self.set_carry(carry); 8} // cp (hl)
            0xbf => { self.set_z(true); self.set_carry(false); 4 } // cp a
            0xc3 => { self.pc = mmu.read_word(self.pc); 16 } // jp nn
            0xc4 => { let address = self.read_word_and_inc_pc(mmu); if !self.z() { self.sp = self.sp - 2; mmu.write_word(self.sp, self.pc); self.pc = address; } 12} // call nz,nn
            0xcc => { let address = self.read_word_and_inc_pc(mmu); if self.z() { self.sp = self.sp - 2; mmu.write_word(self.sp, self.pc); self.pc = address; } 12} // call z,nn
            0xd4 => { let address = self.read_word_and_inc_pc(mmu); if !self.carry() { self.sp = self.sp - 2; mmu.write_word(self.sp, self.pc); self.pc = address; } 12} // call nc,nn
            0xdc => { let address = self.read_word_and_inc_pc(mmu); if self.carry() { self.sp = self.sp - 2; mmu.write_word(self.sp, self.pc); self.pc = address; } 12} // call c,nn
            0xee => { self.a = self.a ^ self.read_and_inc_pc(mmu); let z = self.a == 0; self.set_z(z); self.set_carry(false); 8 } // xor nn
            0xfe => { let val = self.read_and_inc_pc(mmu); let z = self.a == val; self.set_z(z); let carry = self.a < val; self.set_carry(carry); 8} // cp n
            _ => fail!("Unimplemented instruction {:x} at {:x}", instruction, self.pc)
        }
    }

    fn read_and_inc_pc(&mut self, mmu: &Mmu) -> u8 {
        let byte = mmu.read(self.pc);
        self.pc += 1;
        byte
    }

    fn read_word_and_inc_pc(&mut self, mmu: &Mmu) -> u16 {
        let byte = mmu.read_word(self.pc);
        self.pc += 2;
        byte
    }

    fn read_hl(&self, mmu: &Mmu) -> u8 {
        mmu.read((self.h as u16 * 256) + self.l as u16)
    }

    fn write_hl(&self, mmu: &mut Mmu, val: u8) {
        mmu.write((self.h as u16 * 256) + self.l as u16, val);
    }

    fn decrement_hl(&mut self) {
        if self.l == 0x00 {
            self.h = self.h - 1;
            self.l = 0xff;
        } else {
            self.l = self.l - 1;
        }
    }

    fn increment_hl(&mut self) {
        if self.l == 0xff {
            self.h = self.h + 1;
            self.l = 0x00;
        } else {
            self.l = self.l + 1;
        }
    }

    fn z(&self) -> bool {
        self.f & Z_FLAG == Z_FLAG
    }

    fn set_z(&mut self, val: bool) {
        if val {
            self.f = self.f | Z_FLAG;
        } else {
            self.f = self.f ^ Z_FLAG;
        }
    }

    fn carry(&self) -> bool {
        self.c & C_FLAG == C_FLAG
    }

    fn set_carry(&mut self, val: bool) {
        if val {
            self.f = self.f | C_FLAG;
        } else {
            self.f = self.f ^ C_FLAG;
        }
    }

    fn jr(&mut self, distance: u8) {
        if distance & 0x80 != 0x00 {
            let distance = (distance - 1) ^ 0xff;
            self.pc = self.pc - distance as u16;
        } else {
            self.pc = self.pc + distance as u16;
        }
    }

    pub fn as_string(self) -> String {
        format!("pc: {:x} sp: {:x} a: {:x} b: {:x} c: {:x} d: {:x} e: {:x} h: {:x} l: {:x} carry: {}, z: {}",
                 self.pc, self.sp, self.a, self.b, self.c, self.d, self.e, self.h, self.l, self.carry(), self.z())
    }
}
