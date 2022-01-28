use crate::cpu::Cycles;
use crate::mmu::Mapper;

pub fn ld_r_n(r: &mut u8, n: u8) -> Cycles {
  *r = n;
  Cycles(8)
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

pub fn ld_hlptr_dec_a(h: &mut u8, l: &mut u8, a: u8, mapper: &mut Mapper) -> Cycles {
  mapper.write(hl(*h, *l), a);
  decrement_hl(h, l);
  Cycles(8)
}

fn hl(h: u8, l: u8) -> u16 {
  ((h as u16) << 8) + l as u16
}

fn decrement_hl(h: &mut u8, l: &mut u8) {
  if *l == 0 {
    *h -= 1;
    *l = 0xff;
  } else {
    *l -= 1;
  }
}
