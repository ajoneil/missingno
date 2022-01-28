mod arithmetic_logic;
mod control;
mod jump;
mod load;

pub use crate::ops::arithmetic_logic::*;
pub use crate::ops::control::*;
pub use crate::ops::jump::*;
pub use crate::ops::load::*;

fn rr(r1: u8, r2: u8) -> u16 {
  ((r1 as u16) << 8) + r2 as u16
}
