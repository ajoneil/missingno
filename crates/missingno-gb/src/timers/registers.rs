#[derive(Debug)]
pub enum Register {
    Divider,
    Counter,
    Modulo,
    Control,
}

#[derive(Clone)]
pub struct Control(pub u8);

impl Control {
    pub fn enabled(&self) -> bool {
        self.0 & 0b100 != 0
    }

    pub fn selected_bit(&self) -> u16 {
        match self.0 & 0b11 {
            0b00 => 1 << 7,
            0b01 => 1 << 1,
            0b10 => 1 << 3,
            0b11.. => 1 << 5,
        }
    }
}
