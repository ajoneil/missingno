pub struct Joypad {
    read_buttons: bool,
    read_dpad: bool,
}

impl Joypad {
    const UNUSED: u8 = 0b1100_0000;
    const READ_BUTTONS: u8 = 0b0010_0000;
    const READ_DPAD: u8 = 0b0001_0000;

    pub fn new() -> Self {
        Self {
            read_buttons: false,
            read_dpad: false,
        }
    }

    pub fn read_register(&self) -> u8 {
        let mut value = Self::UNUSED;

        // Bits are weirdly inverted for joypad
        if !self.read_buttons {
            value |= Self::READ_BUTTONS;
        }
        if !self.read_dpad {
            value |= Self::READ_DPAD;
        }

        // Nothing pressed
        value | 0xf
    }

    pub fn write_register(&mut self, value: u8) {
        self.read_buttons = value & Self::READ_BUTTONS == 0;
        self.read_dpad = value & Self::READ_DPAD == 0;
    }
}
