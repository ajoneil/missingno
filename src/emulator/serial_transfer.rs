use bitflags::bitflags;

pub struct Registers {
    pub data: u8,
    pub control: Control,
}

impl Registers {
    pub fn new() -> Self {
        Registers {
            data: 0,
            control: Control::from_bits_retain(0x7e),
        }
    }
}

pub enum Register {
    Data,
    Control,
}

// pub enum Clock {
//     Internal,
//     External,
// }

bitflags! {
    #[derive(Copy, Clone)]
    pub struct Control: u8 {
        const ENABLE         = 0b10000000;
        const INTERNAL_CLOCK = 0b00000001;

        const _OTHER = !0;
    }
}

// impl Control {
//     pub fn enabled(self) -> bool {
//         self.contains(Control::ENABLE)
//     }

//     pub fn clock(self) -> Clock {
//         if self.contains(Control::INTERNAL_CLOCK) {
//             Clock::Internal
//         } else {
//             Clock::External
//         }
//     }
// }
