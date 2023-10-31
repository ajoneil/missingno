use crate::{cpu::Cycles, mmu::Mapper};

use super::rr;

pub fn jp_nn(pc: &mut u16, nn: u16) -> Cycles {
    *pc = nn;
    Cycles(16)
}

pub fn jp_hl(pc: &mut u16, h: u8, l: u8) -> Cycles {
    *pc = rr(h, l);
    Cycles(4)
}

pub fn jp_f_nn(pc: &mut u16, f: bool, nn: u16) -> Cycles {
    if f {
        *pc = nn;
        Cycles(16)
    } else {
        Cycles(12)
    }
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

fn call(pc: &mut u16, sp: &mut u16, address: u16, mapper: &mut Mapper) {
    *sp -= 2;
    mapper.write_word(*sp, *pc);
    *pc = address;
}

pub fn call_nn(pc: &mut u16, sp: &mut u16, nn: u16, mapper: &mut Mapper) -> Cycles {
    call(pc, sp, nn, mapper);
    Cycles(24)
}

pub fn call_f_nn(pc: &mut u16, sp: &mut u16, f: bool, nn: u16, mapper: &mut Mapper) -> Cycles {
    if f {
        call(pc, sp, nn, mapper);
        Cycles(24)
    } else {
        Cycles(12)
    }
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

pub fn reti(pc: &mut u16, sp: &mut u16, ime: &mut bool, mapper: &Mapper) -> Cycles {
    *pc = mapper.read_word(*sp);
    *sp += 2;
    println!("master interrupt enabled");
    *ime = true;
    Cycles(16)
}

pub fn rst_n(pc: &mut u16, sp: &mut u16, n: u8, mapper: &mut Mapper) -> Cycles {
    call(pc, sp, n as u16, mapper);
    Cycles(16)
}
