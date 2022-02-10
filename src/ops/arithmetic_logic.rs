use crate::cpu::{Cycles, Flags};
use crate::mmu::Mapper;
use crate::ops::rr;

pub fn add_a_r(a: &mut u8, r: u8, f: &mut Flags) -> Cycles {
  let res = *a as u16 + r as u16;
  *a = (res & 0xff) as u8;

  f.set(Flags::Z, *a == 0);
  f.remove(Flags::N);
  f.set(Flags::H, (((*a & 0xf) + (r & 0xf)) & 0x10) != 0);
  f.set(Flags::C, res > 0xff);

  Cycles(4)
}

pub fn add_a_n(a: &mut u8, n: u8, f: &mut Flags) -> Cycles {
  let res = *a as u16 + n as u16;
  *a = (res & 0xff) as u8;

  f.set(Flags::Z, *a == 0);
  f.remove(Flags::N);
  f.set(Flags::H, (((*a & 0xf) + (n & 0xf)) & 0x10) != 0);
  f.set(Flags::C, res > 0xff);

  Cycles(8)
}

pub fn add_a_hlptr(a: &mut u8, h: u8, l: u8, f: &mut Flags, mapper: &Mapper) -> Cycles {
  let val = mapper.read(rr(h, l));
  let res = *a as u16 + val as u16;
  *a = (res & 0xff) as u8;

  f.set(Flags::Z, *a == 0);
  f.remove(Flags::N);
  f.set(Flags::H, (((*a & 0xf) + (val & 0xf)) & 0x10) != 0);
  f.set(Flags::C, res > 0xff);

  Cycles(8)
}

pub fn adc_a_r(a: &mut u8, r: u8, f: &mut Flags) -> Cycles {
  let carry = if f.contains(Flags::C) { 1 } else { 0 };
  let res = *a as u16 + r as u16 + carry;
  *a = (res & 0xff) as u8;

  f.set(Flags::Z, *a == 0);
  f.remove(Flags::N);
  f.set(
    Flags::H,
    (((*a & 0xf) + (r & 0xf) + carry as u8) & 0x10) != 0,
  );
  f.set(Flags::C, res > 0xff);

  Cycles(4)
}

pub fn adc_a_n(a: &mut u8, n: u8, f: &mut Flags) -> Cycles {
  let carry = if f.contains(Flags::C) { 1 } else { 0 };
  let res = *a as u16 + n as u16 + carry;
  *a = (res & 0xff) as u8;

  f.set(Flags::Z, *a == 0);
  f.remove(Flags::N);
  f.set(
    Flags::H,
    (((*a & 0xf) + (n & 0xf) + carry as u8) & 0x10) != 0,
  );
  f.set(Flags::C, res > 0xff);

  Cycles(8)
}

pub fn adc_a_hlptr(a: &mut u8, h: u8, l: u8, f: &mut Flags, mapper: &Mapper) -> Cycles {
  let val = mapper.read(rr(h, l));
  let carry = if f.contains(Flags::C) { 1 } else { 0 };
  let res = *a as u16 + val as u16 + carry;
  *a = (res & 0xff) as u8;

  f.set(Flags::Z, *a == 0);
  f.remove(Flags::N);
  f.set(
    Flags::H,
    (((*a & 0xf) + (val & 0xf) + carry as u8) & 0x10) != 0,
  );
  f.set(Flags::C, res > 0xff);

  Cycles(8)
}

pub fn sub_r(a: &mut u8, r: u8, f: &mut Flags) -> Cycles {
  let res = *a as i16 - r as i16;
  *a = (res & 0xff) as u8;

  f.set(Flags::Z, *a == 0);
  f.insert(Flags::N);
  f.set(Flags::H, (*a & 0xf) < (r & 0xf));
  f.set(Flags::C, res < 0);

  Cycles(4)
}

pub fn sub_n(a: &mut u8, n: u8, f: &mut Flags) -> Cycles {
  let res = *a as i16 - n as i16;
  *a = (res & 0xff) as u8;

  f.set(Flags::Z, *a == 0);
  f.insert(Flags::N);
  f.set(Flags::H, (*a & 0xf) < (n & 0xf));
  f.set(Flags::C, res < 0);

  Cycles(8)
}

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
  let val = mapper.read(rr(h, l));
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
