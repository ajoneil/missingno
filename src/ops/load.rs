use crate::cpu::Cycles;

pub fn ld_r_n(r: &mut u8, n: u8) -> Cycles {
  *r = n;
  Cycles(8)
}

pub fn ld_rr_nn(r1: &mut u8, r2: &mut u8, nn: u16) -> Cycles {
  *r1 = (nn & 0xff) as u8;
  *r2 = ((nn & 0xff00) >> 8) as u8;
  Cycles(12)
}

pub fn ld_sp_nn(sp: &mut u16, nn: u16) -> Cycles {
  *sp = nn;
  Cycles(12)
}

// pub fn ld_hlptr_dec_a(&mut self, mmu: &mut Mmu, video: &mut Video) -> u32 {
//   self.write_hl(mmu, self.a, video);
//   self.decrement_hl();
//   8
// }

// fn write_hl(n: u8)
