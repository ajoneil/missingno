use crate::{cpu::Cycles, mmu::Mapper};

pub fn jp_nn(pc: &mut u16, nn: u16) -> Cycles {
    *pc = nn;
    Cycles(16)
}

pub fn jr(pc: &mut u16, distance: u8) -> Cycles {
    if distance & 0x80 != 0x00 {
        let distance = (distance - 1) ^ 0xff;
        *pc -= distance as u16;
    } else {
        *pc += distance as u16;
    }

    Cycles(12)
}

pub fn jr_if(pc: &mut u16, distance: u8, condition: bool) -> Cycles {
    if condition {
        jr(pc, distance)
    } else {
        Cycles(8)
    }
}

pub fn call_nn(pc: &mut u16, sp: &mut u16, nn: u16, mapper: &mut Mapper) -> Cycles {
    *sp -= 2;
    mapper.write_word(*sp, *pc);
    *pc = nn;
    Cycles(24)
}

pub fn ret(pc: &mut u16, sp: &mut u16, mapper: &Mapper) -> Cycles {
    *pc = mapper.read_word(*sp);
    *sp += 2;
    Cycles(16)
}

pub fn ret_f(pc: &mut u16, sp: &mut u16, f: bool, mapper: &Mapper) -> Cycles {
    if f {
        *pc = mapper.read_word(*sp);
        *sp += 2;
        Cycles(20)
    } else {
        Cycles(8)
    }
}
