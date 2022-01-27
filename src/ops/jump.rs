use crate::cpu::{Cycles, Flags};

pub fn jp_nn(pc: &mut u16, nn: u16) -> Cycles {
  *pc = nn;
  Cycles(16)
}
