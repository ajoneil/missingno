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
            0x00 => 4,                                       // nop
            0xc3 => { self.pc = mmu.read_word(self.pc); 16 } // jp nn
            _ => fail!("Unimplemented instruction {:x} at {:x}", instruction, self.pc)
        }
    }

    pub fn read_and_inc_pc(&mut self, mmu: &Mmu) -> u8 {
        let byte = mmu.read(self.pc);
        self.pc += 1;
        byte
    }
}
