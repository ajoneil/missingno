use crate::cpu::{Cycles, Flags};
use crate::mmu::Mapper;
use crate::ops::hl;

pub fn xor_r(r: u8, a: &mut u8, f: &mut Flags) -> Cycles {
  *a = *a ^ r;
  *f = if *a == 0 { Flags::Z } else { Flags::empty() };

  Cycles(4)
}

pub fn cp_r(r: u8, a: u8, f: &mut Flags) -> Cycles {
  f.set(Flags::Z, a == r);
  f.insert(Flags::N);
  f.set(Flags::C, r > a);

  Cycles(4)
}

pub fn cp_n(n: u8, a: u8, f: &mut Flags) -> Cycles {
  f.set(Flags::Z, a == n);
  f.insert(Flags::N);
  f.set(Flags::C, n > a);

  Cycles(8)
}

pub fn cp_hlptr(h: u8, l: u8, a: u8, f: &mut Flags, mapper: &Mapper) -> Cycles {
  let val = mapper.read(hl(h, l));
  f.set(Flags::Z, a == val);
  f.insert(Flags::N);
  f.set(Flags::C, val > a);

  Cycles(8)
}

pub fn dec_r(r: &mut u8, f: &mut Flags) -> Cycles {
  *r = if *r == 0 { 0xff } else { *r - 1 };
  f.set(Flags::Z, *r == 0);
  f.insert(Flags::N);

  Cycles(4)
}
