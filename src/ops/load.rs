use crate::cpu::Cycles;
use crate::mmu::Mapper;
use crate::ops::rr;

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

pub fn ld_hlptr_dec_a(h: &mut u8, l: &mut u8, a: u8, mapper: &mut Mapper) -> Cycles {
  mapper.write(rr(*h, *l), a);
  decrement_hl(h, l);
  Cycles(8)
}

fn decrement_hl(h: &mut u8, l: &mut u8) {
  if *l == 0 {
    *h -= 1;
    *l = 0xff;
  } else {
    *l -= 1;
  }
}

pub fn ld_rr_nn(r1: &mut u8, r2: &mut u8, nn: u16) -> Cycles {
  *r1 = ((nn & 0xff00) >> 8) as u8;
  *r2 = (nn & 0xff) as u8;
  Cycles(12)
}

pub fn ld_sp_nn(sp: &mut u16, nn: u16) -> Cycles {
  *sp = nn;
  Cycles(12)
}
