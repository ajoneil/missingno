use bitflags::bitflags;

use super::{Audio, volume::Volume};

#[derive(PartialEq, Eq)]
pub enum Register {
    Control,
    Panning,
    Volume,
}

impl Audio {
    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Control => {
                if self.enabled {
                    let mut value = ControlFlags::AUDIO_ENABLE;
                    value.set(
                        ControlFlags::CHANNEL_1_ON,
                        self.channels.ch1.channel.enabled,
                    );
                    value.set(
                        ControlFlags::CHANNEL_2_ON,
                        self.channels.ch2.channel.enabled,
                    );
                    value.set(
                        ControlFlags::CHANNEL_3_ON,
                        self.channels.ch3.channel.enabled,
                    );
                    value.set(
                        ControlFlags::CHANNEL_4_ON,
                        self.channels.ch4.channel.enabled,
                    );

                    value.bits()
                } else {
                    0x00
                }
            }
            Register::Panning => {
                let mut value = PanFlags::empty();
                value.set(
                    PanFlags::CHANNEL_1_LEFT,
                    self.channels().ch1.channel.output_left,
                );
                value.set(
                    PanFlags::CHANNEL_1_RIGHT,
                    self.channels().ch1.channel.output_right,
                );
                value.set(
                    PanFlags::CHANNEL_2_LEFT,
                    self.channels().ch2.channel.output_left,
                );
                value.set(
                    PanFlags::CHANNEL_2_RIGHT,
                    self.channels().ch2.channel.output_right,
                );
                value.set(
                    PanFlags::CHANNEL_3_LEFT,
                    self.channels().ch3.channel.output_left,
                );
                value.set(
                    PanFlags::CHANNEL_3_RIGHT,
                    self.channels().ch3.channel.output_right,
                );
                value.set(
                    PanFlags::CHANNEL_4_LEFT,
                    self.channels().ch4.channel.output_left,
                );
                value.set(
                    PanFlags::CHANNEL_4_RIGHT,
                    self.channels().ch4.channel.output_right,
                );

                value.bits()
            }
            Register::Volume => (self.volume_left.0 << 4) & self.volume_right.0,
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        if !self.enabled && register != Register::Control {
            panic!("Can't write to audio register when audio is disabled");
        }

        match register {
            Register::Control => {
                if ControlFlags::from_bits_retain(value).contains(ControlFlags::AUDIO_ENABLE) {
                    self.enabled = true;
                } else {
                    self.enabled = false;
                    self.channels.ch1.reset();
                    self.channels.ch2.reset();
                    self.channels.ch3.reset();
                    self.channels.ch4.reset();
                }
            }
            Register::Panning => {
                let value = PanFlags::from_bits_truncate(value);
                self.channels.ch1.channel.output_left = value.contains(PanFlags::CHANNEL_1_LEFT);
                self.channels.ch1.channel.output_right = value.contains(PanFlags::CHANNEL_1_RIGHT);
                self.channels.ch2.channel.output_left = value.contains(PanFlags::CHANNEL_2_LEFT);
                self.channels.ch2.channel.output_right = value.contains(PanFlags::CHANNEL_2_RIGHT);
                self.channels.ch3.channel.output_left = value.contains(PanFlags::CHANNEL_3_LEFT);
                self.channels.ch3.channel.output_right = value.contains(PanFlags::CHANNEL_3_RIGHT);
                self.channels.ch4.channel.output_left = value.contains(PanFlags::CHANNEL_4_LEFT);
                self.channels.ch4.channel.output_right = value.contains(PanFlags::CHANNEL_4_RIGHT);
            }
            Register::Volume => {
                self.volume_left = Volume((value >> 4) & 0b111);
                self.volume_right = Volume(value & 0b111);
            }
        }
    }
}

bitflags! {
    pub struct ControlFlags: u8 {
        const AUDIO_ENABLE  = 0b1000_0000;
        const CHANNEL_4_ON  = 0b0000_1000;
        const CHANNEL_3_ON  = 0b0000_0100;
        const CHANNEL_2_ON  = 0b0000_0010;
        const CHANNEL_1_ON  = 0b0000_0001;
    }
}

bitflags! {
    pub struct PanFlags : u8 {
        const CHANNEL_4_LEFT  = 0b1000_0000;
        const CHANNEL_3_LEFT  = 0b0100_0000;
        const CHANNEL_2_LEFT  = 0b0010_0000;
        const CHANNEL_1_LEFT  = 0b0001_0000;
        const CHANNEL_4_RIGHT = 0b0000_1000;
        const CHANNEL_3_RIGHT = 0b0000_0100;
        const CHANNEL_2_RIGHT = 0b0000_0010;
        const CHANNEL_1_RIGHT = 0b0000_0001;
    }
}
