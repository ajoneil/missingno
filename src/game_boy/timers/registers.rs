#[derive(Debug)]
pub enum Register {
    Divider,
    Counter,
    Modulo,
    Control,
}

use nanoserde::{DeRon, SerRon};

#[derive(Clone, SerRon, DeRon)]
pub struct Control(pub u8);

impl Control {
    pub fn enabled(&self) -> bool {
        self.0 & 0b100 != 0
    }

    pub fn cycle_interval(&self) -> u32 {
        match self.0 & 0b11 {
            0b00 => 1024,
            0b01 => 16,
            0b10 => 64,
            0b11.. => 256,
        }
    }
}
