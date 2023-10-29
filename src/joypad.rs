pub struct Joypad {
    control_bits: u8,
}

enum Enabled {
    Buttons,
    Dpad,
    Neither,
}

impl Joypad {
    pub fn new() -> Self {
        Self { control_bits: 0 }
    }

    pub fn read(&self) -> u8 {
        // 0b11000000
        //     & self.control_bits << 4
        //     & match self.enabled() {
        //         Enabled::Buttons => self.button_bits(),
        //         Enabled::Dpad => self.dpad_bits(),
        //         Enabled::Neither => 0b1111,
        //     }
        0xef
    }

    pub fn write(&mut self, val: u8) {
        self.control_bits = (val >> 4) & 0b11;
    }

    fn enabled(&self) -> Enabled {
        if self.control_bits & 0b10 != 0 {
            Enabled::Buttons
        } else if self.control_bits & 0b1 != 0 {
            Enabled::Dpad
        } else {
            Enabled::Neither
        }
    }

    fn button_bits(&self) -> u8 {
        0b1111
    }

    fn dpad_bits(&self) -> u8 {
        0b1111
    }
}
