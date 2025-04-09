use crate::emulation::cpu::{Cycles, Flags};
use crate::emulation::mmu::Mapper;
use crate::emulation::ops::rr;

pub fn ld_r_r(rw: &mut u8, rr: u8) -> Cycles {
    *rw = rr;
    Cycles(4)
}

pub fn ld_r_n(r: &mut u8, n: u8) -> Cycles {
    *r = n;
    Cycles(8)
}

pub fn ld_r_rrptr(rw: &mut u8, rr1: u8, rr2: u8, mapper: &Mapper) -> Cycles {
    *rw = mapper.read(rr(rr1, rr2));
    Cycles(8)
}

pub fn ld_hlptr_r(h: u8, l: u8, r: u8, mapper: &mut Mapper) -> Cycles {
    mapper.write(rr(h, l), r);
    Cycles(8)
}

pub fn ld_hlptr_n(h: u8, l: u8, n: u8, mapper: &mut Mapper) -> Cycles {
    mapper.write(rr(h, l), n);
    Cycles(12)
}

pub fn ld_a_nnptr(a: &mut u8, nn: u16, mapper: &Mapper) -> Cycles {
    *a = mapper.read(nn);
    Cycles(16)
}

pub fn ld_rrptr_a(r1: u8, r2: u8, a: u8, mapper: &mut Mapper) -> Cycles {
    mapper.write(rr(r1, r2), a);
    Cycles(8)
}

pub fn ld_nnptr_a(nn: u16, a: u8, mapper: &mut Mapper) -> Cycles {
    mapper.write(nn, a);
    Cycles(16)
}

pub fn ld_a_nhptr(a: &mut u8, n: u8, mapper: &Mapper) -> Cycles {
    *a = mapper.read(0xff00 + n as u16);
    Cycles(12)
}

pub fn ld_nhptr_a(n: u8, a: u8, mapper: &mut Mapper) -> Cycles {
    mapper.write(0xff00 + n as u16, a);
    Cycles(12)
}

pub fn ld_a_chptr(a: &mut u8, c: u8, mapper: &Mapper) -> Cycles {
    *a = mapper.read(0xff00 + c as u16);
    Cycles(8)
}

pub fn ld_chptr_a(c: u8, a: u8, mapper: &mut Mapper) -> Cycles {
    mapper.write(0xff00 + c as u16, a);
    Cycles(8)
}

pub fn ld_hlptr_inc_a(h: &mut u8, l: &mut u8, a: u8, mapper: &mut Mapper) -> Cycles {
    mapper.write(rr(*h, *l), a);
    increment_hl(h, l);
    Cycles(8)
}

pub fn ld_a_hlptr_inc(a: &mut u8, h: &mut u8, l: &mut u8, mapper: &Mapper) -> Cycles {
    *a = mapper.read(rr(*h, *l));
    increment_hl(h, l);
    Cycles(8)
}

pub fn ld_hlptr_dec_a(h: &mut u8, l: &mut u8, a: u8, mapper: &mut Mapper) -> Cycles {
    mapper.write(rr(*h, *l), a);
    decrement_hl(h, l);
    Cycles(8)
}

pub fn ld_a_hlptr_dec(a: &mut u8, h: &mut u8, l: &mut u8, mapper: &Mapper) -> Cycles {
    *a = mapper.read(rr(*h, *l));
    decrement_hl(h, l);
    Cycles(8)
}

fn increment_hl(h: &mut u8, l: &mut u8) {
    if *l == 0xff {
        *h = if *h == 0xff { 0 } else { *h + 1 };
        *l = 0;
    } else {
        *l += 1;
    }
}

fn decrement_hl(h: &mut u8, l: &mut u8) {
    if *l == 0 {
        *h -= 1;
        *l = 0xff;
    } else {
        *l -= 1;
    }
}

pub fn ld_nnptr_sp(nn: u16, sp: u16, mapper: &mut Mapper) -> Cycles {
    mapper.write_word(nn, sp);
    Cycles(20)
}

pub fn ld_sp_hl(sp: &mut u16, h: u8, l: u8) -> Cycles {
    *sp = rr(h, l);
    Cycles(8)
}

pub fn push_rr(r1: u8, r2: u8, sp: &mut u16, mapper: &mut Mapper) -> Cycles {
    *sp -= 2;
    mapper.write_word(*sp, rr(r1, r2));
    Cycles(16)
}

pub fn pop_rr(r1: &mut u8, r2: &mut u8, sp: &mut u16, mapper: &mut Mapper) -> Cycles {
    *r2 = mapper.read(*sp);
    *r1 = mapper.read(*sp + 1);
    *sp += 2;
    Cycles(16)
}

pub fn pop_af(a: &mut u8, f: &mut Flags, sp: &mut u16, mapper: &mut Mapper) -> Cycles {
    *a = mapper.read(*sp);
    *f = Flags::from_bits_truncate(mapper.read(*sp + 1));
    *sp += 2;
    Cycles(16)
}
