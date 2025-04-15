use crate::emulator::cpu::cycles::Cycles;

#[derive(Debug)]
pub enum Register {
    Divider,
    Counter,
    Modulo,
    Control,
}

pub struct Control(pub u8);

impl Control {
    pub fn enabled(&self) -> bool {
        self.0 & 0b100 != 0
    }

    pub fn interval(&self) -> Cycles {
        match self.0 & 0b11 {
            0b00 => Cycles(1024),
            0b01 => Cycles(16),
            0b10 => Cycles(64),
            0b11.. => Cycles(256),
        }
    }
}
