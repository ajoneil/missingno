use crate::cpu::Cycles;

pub fn set_n_r(n: u8, r: &mut u8) -> Cycles {
    *r |= 1 << n;
    Cycles(8)
}

pub fn res_n_r(n: u8, r: &mut u8) -> Cycles {
    *r ^= 1 << n;
    Cycles(8)
}
