use crate::cpu::{Cycles, Flags};

pub fn rlca(a: &mut u8, f: &mut Flags) -> Cycles {
  let (shifted_a, overflowed) = a.overflowing_shl(1);

  *a = if overflowed { shifted_a & 1 } else { shifted_a };
  *f = if overflowed { Flags::C } else { Flags::empty() };

  Cycles(4)
}

pub fn rla(a: &mut u8, f: &mut Flags) -> Cycles {
  let (shifted_a, overflowed) = a.overflowing_shl(1);

  *a = if f.contains(Flags::C) {
    shifted_a & 1
  } else {
    shifted_a
  };
  *f = if overflowed { Flags::C } else { Flags::empty() };

  Cycles(4)
}

pub fn rrca(a: &mut u8, f: &mut Flags) -> Cycles {
  let (shifted_a, overflowed) = a.overflowing_shr(1);

  *a = if overflowed {
    shifted_a & 0xf0
  } else {
    shifted_a
  };
  *f = if overflowed { Flags::C } else { Flags::empty() };

  Cycles(4)
}

pub fn rra(a: &mut u8, f: &mut Flags) -> Cycles {
  let (shifted_a, overflowed) = a.overflowing_shr(1);

  *a = if f.contains(Flags::C) {
    shifted_a & 0xf0
  } else {
    shifted_a
  };
  *f = if overflowed { Flags::C } else { Flags::empty() };

  Cycles(4)
}
