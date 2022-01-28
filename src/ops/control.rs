use crate::cpu::Cycles;

pub fn nop() -> Cycles {
  Cycles(4)
}

pub fn di(ime: &mut bool) -> Cycles {
  *ime = false;
  Cycles(4)
}

pub fn ei(ime: &mut bool) -> Cycles {
  *ime = true;
  Cycles(4)
}
