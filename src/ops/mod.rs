mod arithmetic_logic;
mod control;
mod jump;
mod load;
mod rotate_shift;

pub use crate::ops::arithmetic_logic::*;
pub use crate::ops::control::*;
pub use crate::ops::jump::*;
pub use crate::ops::load::*;
pub use crate::ops::rotate_shift::*;

fn rr(r1: u8, r2: u8) -> u16 {
  ((r1 as u16) << 8) + r2 as u16
}

fn set_rr(r1: &mut u8, r2: &mut u8, val: u16) {
  *r1 = (val >> 8) as u8;
  *r2 = (val & 0xff) as u8;
}
