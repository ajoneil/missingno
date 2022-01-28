use crate::cpu::Cycles;

pub fn jp_nn(pc: &mut u16, nn: u16) -> Cycles {
  *pc = nn;
  Cycles(16)
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
