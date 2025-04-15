use super::Enabled;
use crate::emulator::audio::channels::registers::VolumeAndEnvelope;

#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    VolumeAndEnvelope,
    Control,
}

pub struct NoiseChannel {
    pub enabled: Enabled,
    pub volume_and_envelope: VolumeAndEnvelope,
    pub length_enabled: bool,
}

impl Default for NoiseChannel {
    fn default() -> Self {
        Self {
            enabled: Enabled {
                enabled: false,
                output_left: true,
                output_right: true,
            },
            volume_and_envelope: VolumeAndEnvelope(0),
            length_enabled: true,
        }
    }
}

impl NoiseChannel {
    pub fn reset(&mut self) {
        self.enabled = Enabled::disabled();
        self.volume_and_envelope = VolumeAndEnvelope(0);
        self.length_enabled = false;
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::VolumeAndEnvelope => self.volume_and_envelope.0,
            Register::Control => Control::read(self.length_enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::VolumeAndEnvelope => self.volume_and_envelope = VolumeAndEnvelope(value),
            Register::Control => {
                let value = Control(value);
                self.length_enabled = value.enable_length();
                if value.trigger() {
                    self.trigger();
                }
            }
        }
    }

    pub fn trigger(&mut self) {
        // TODO audio
    }
}

struct Control(pub u8);

impl Control {
    const LENGTH: u8 = 0b0100_0000;

    pub fn read(length_enabled: bool) -> u8 {
        if length_enabled {
            0xff
        } else {
            0xff ^ Self::LENGTH
        }
    }

    pub fn trigger(&self) -> bool {
        self.0 & 0b1000_0000 != 0
    }

    pub fn enable_length(&self) -> bool {
        self.0 & Self::LENGTH != 0
    }
}
