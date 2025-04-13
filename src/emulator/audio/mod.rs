pub mod channels;
pub mod control;

use channels::{
    Channels, noise::NoiseChannel, pulse::PulseChannel, pulse_sweep::PulseSweepChannel,
    wave::WaveChannel,
};
use control::ControlFlags;

pub enum Register {
    Control,
}

pub struct Audio {
    enabled: bool,
    channels: Channels,
}

impl Audio {
    pub fn new() -> Self {
        Self {
            enabled: true,
            channels: Channels {
                ch1: PulseSweepChannel::default(),
                ch2: PulseChannel::default(),
                ch3: WaveChannel::default(),
                ch4: NoiseChannel::default(),
            },
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn channels(&self) -> &Channels {
        &self.channels
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Control => {
                if self.enabled {
                    let mut value = ControlFlags::AUDIO_ENABLE;
                    value.set(ControlFlags::CHANNEL_1_ON, self.channels.ch1.enabled());
                    value.set(ControlFlags::CHANNEL_2_ON, self.channels.ch2.enabled());
                    value.set(ControlFlags::CHANNEL_3_ON, self.channels.ch3.enabled());
                    value.set(ControlFlags::CHANNEL_4_ON, self.channels.ch4.enabled());

                    value.bits()
                } else {
                    0x00
                }
            }
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::Control => {
                if ControlFlags::from_bits_retain(value).contains(ControlFlags::AUDIO_ENABLE) {
                    self.enabled = true;
                } else {
                    self.enabled = false;
                    self.channels.ch1.disable();
                    self.channels.ch2.disable();
                    self.channels.ch3.disable();
                    self.channels.ch4.disable();
                }
            }
        }
    }
}
