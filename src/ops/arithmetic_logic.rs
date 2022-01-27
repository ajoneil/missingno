use crate::cpu::{Cycles, Flags};

pub fn dec_r(r: &mut u8, f: &mut Flags) -> Cycles {
  *r = if *r == 0 { 0xff } else { *r - 1 };

  f.set(Flags::Z, *r == 0);
  f.insert(Flags::N);
  Cycles(4)
}

pub fn xor_r(r: u8, a: &mut u8, f: &mut Flags) -> Cycles {
  *a = *a ^ r;
  *f = if *a == 0 { Flags::Z } else { Flags::empty() };
  Cycles(4)
}
