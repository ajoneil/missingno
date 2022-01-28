mod arithmetic_logic;
mod control;
mod jump;
mod load;

pub use crate::ops::arithmetic_logic::*;
pub use crate::ops::control::*;
pub use crate::ops::jump::*;
pub use crate::ops::load::*;

fn hl(h: u8, l: u8) -> u16 {
  ((h as u16) << 8) + l as u16
}
