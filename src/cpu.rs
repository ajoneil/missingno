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
static H_FLAG: u8 = 0b00010000;
static C_FLAG: u8 = 0b00001000;

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

    pub fn step(&mut self, mmu: &Mmu) -> int {
        let instruction = self.read_and_inc_pc(mmu);
        match instruction {
            0x00 => 4, // nop
            0x01 => { self.b = self.read_and_inc_pc(mmu); self.c = self.read_and_inc_pc(mmu); 12 } // ld bc,nn
            0x11 => { self.d = self.read_and_inc_pc(mmu); self.e = self.read_and_inc_pc(mmu); 12 } // ld de,nn
            0x21 => { self.h = self.read_and_inc_pc(mmu); self.l = self.read_and_inc_pc(mmu); 12 } // ld hl,nn
            0x31 => { self.sp = self.read_word_and_inc_pc(mmu); 12 } // ld sp,nn
            0xa8 => { self.a = self.a ^ self.b; let a = self.a; self.set_z(a); self.clear_n(); self.clear_h(); self.clear_c(); 4 } // xor b
            0xa9 => { self.a = self.a ^ self.c; let a = self.a; self.set_z(a); self.clear_n(); self.clear_h(); self.clear_c(); 4 } // xor c
            0xaa => { self.a = self.a ^ self.d; let a = self.a; self.set_z(a); self.clear_n(); self.clear_h(); self.clear_c(); 4 } // xor d
            0xab => { self.a = self.a ^ self.e; let a = self.a; self.set_z(a); self.clear_n(); self.clear_h(); self.clear_c(); 4 } // xor e
            0xac => { self.a = self.a ^ self.h; let a = self.a; self.set_z(a); self.clear_n(); self.clear_h(); self.clear_c(); 4 } // xor h
            0xad => { self.a = self.a ^ self.l; let a = self.a; self.set_z(a); self.clear_n(); self.clear_h(); self.clear_c(); 4 } // xor l
            0xae => { self.a = self.a ^ self.read_hl(mmu); let a = self.a; self.set_z(a); self.clear_n(); self.clear_h(); self.clear_c(); 8 } // xor (hl)
            0xaf => { self.a = self.a ^ self.a; let a = self.a; self.set_z(a); self.clear_n(); self.clear_h(); self.clear_c(); 4 } // xor a
            0xc3 => { self.pc = mmu.read_word(self.pc); 16 } // jp nn
            0xee => { self.a = self.a ^ self.read_and_inc_pc(mmu); let a = self.a; self.set_z(a); self.clear_n(); self.clear_h(); self.clear_c(); 8 } // xor nn
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

    fn set_z(&mut self, val: u8) {
        if val == 0 {
            self.f = self.f | Z_FLAG;
        } else {
            self.f = self.f ^ Z_FLAG;
        }
    }

    fn clear_n(&mut self) {
        self.f = self.f ^ N_FLAG;
    }

    fn clear_h(&mut self) {
        self.f = self.f ^ H_FLAG;
    }

    fn clear_c(&mut self) {
        self.f = self.f & C_FLAG;
    }
}
