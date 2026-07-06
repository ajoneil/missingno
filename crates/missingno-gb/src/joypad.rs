use bitflags::bitflags;

#[derive(Clone)]
pub struct Joypad {
    pub read_buttons: bool,
    pub read_dpad: bool,

    pub pressed: Buttons,
}

#[derive(Eq, PartialEq, Hash, Debug, Clone, Copy)]
pub enum Button {
    Start,
    Select,
    A,
    B,
    DirectionalPad(DirectionalPad),
}

#[derive(Eq, PartialEq, Hash, Debug, Clone, Copy)]
pub enum DirectionalPad {
    Up,
    Down,
    Left,
    Right,
}

bitflags! {
    /// The 8-key button matrix as a bitset — one bit per key, allocation-free.
    #[derive(Clone, Copy)]
    pub struct Buttons: u8 {
        const START  = 1 << 0;
        const SELECT = 1 << 1;
        const A      = 1 << 2;
        const B      = 1 << 3;
        const UP     = 1 << 4;
        const DOWN   = 1 << 5;
        const LEFT   = 1 << 6;
        const RIGHT  = 1 << 7;
    }
}

impl From<Button> for Buttons {
    fn from(button: Button) -> Self {
        match button {
            Button::Start => Buttons::START,
            Button::Select => Buttons::SELECT,
            Button::A => Buttons::A,
            Button::B => Buttons::B,
            Button::DirectionalPad(DirectionalPad::Up) => Buttons::UP,
            Button::DirectionalPad(DirectionalPad::Down) => Buttons::DOWN,
            Button::DirectionalPad(DirectionalPad::Left) => Buttons::LEFT,
            Button::DirectionalPad(DirectionalPad::Right) => Buttons::RIGHT,
        }
    }
}

impl Default for Joypad {
    fn default() -> Self {
        Self::new()
    }
}

impl Joypad {
    const UNUSED: u8 = 0b1100_0000;
    const READ_BUTTONS: u8 = 0b0010_0000;
    const READ_DPAD: u8 = 0b0001_0000;
    const START_DOWN: u8 = 0b0000_1000;
    const SELECT_UP: u8 = 0b0000_0100;
    const B_LEFT: u8 = 0b0000_0010;
    const A_RIGHT: u8 = 0b0000_0001;
    const NONE_PRESSED: u8 = 0xf;

    pub fn new() -> Self {
        Self {
            read_buttons: true,
            read_dpad: true,
            pressed: Buttons::empty(),
        }
    }

    pub fn read_register(&self) -> u8 {
        // Bits are weirdly inverted for joypad
        let mut value = Self::UNUSED | Self::NONE_PRESSED;

        if self.read_buttons {
            if self.pressed.contains(Buttons::START) {
                value &= !Self::START_DOWN;
            }
            if self.pressed.contains(Buttons::SELECT) {
                value &= !Self::SELECT_UP;
            }
            if self.pressed.contains(Buttons::B) {
                value &= !Self::B_LEFT;
            }
            if self.pressed.contains(Buttons::A) {
                value &= !Self::A_RIGHT;
            }
        } else {
            value |= Self::READ_BUTTONS;
        }

        if self.read_dpad {
            if self.pressed.contains(Buttons::DOWN) {
                value &= !Self::START_DOWN;
            }
            if self.pressed.contains(Buttons::UP) {
                value &= !Self::SELECT_UP;
            }
            if self.pressed.contains(Buttons::LEFT) {
                value &= !Self::B_LEFT;
            }
            if self.pressed.contains(Buttons::RIGHT) {
                value &= !Self::A_RIGHT;
            }
        } else {
            value |= Self::READ_DPAD;
        }

        value
    }

    pub fn write_register(&mut self, value: u8) {
        self.read_buttons = value & Self::READ_BUTTONS == 0;
        self.read_dpad = value & Self::READ_DPAD == 0;
    }

    /// The four input lines P10-P13 (low nibble of JOYP). Each bit is
    /// 0 when a selected button on that line is pressed. The Joypad IF
    /// (ULAK) clocks on any 1→0 transition of these lines.
    pub fn input_lines(&self) -> u8 {
        self.read_register() & 0x0F
    }

    pub fn press_button(&mut self, button: Button) {
        self.pressed.insert(button.into());
    }

    pub fn release_button(&mut self, button: Button) {
        self.pressed.remove(button.into());
    }
}
