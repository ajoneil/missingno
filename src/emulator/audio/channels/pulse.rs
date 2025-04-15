use super::{
    Enabled,
    registers::{PeriodHighAndControl, Signed11},
};
use crate::emulator::audio::channels::registers::VolumeAndEnvelope;

#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    VolumeAndEnvelope,
    PeriodLow,
    PeriodHighAndControl,
}

pub struct PulseChannel {
    pub enabled: Enabled,
    pub volume_and_envelope: VolumeAndEnvelope,
    pub length_enabled: bool,
    pub period: Signed11,
}

impl Default for PulseChannel {
    fn default() -> Self {
        Self {
            enabled: Enabled {
                enabled: false,
                output_left: true,
                output_right: true,
            },
            volume_and_envelope: VolumeAndEnvelope(0xf3),
            length_enabled: false,
            period: (-1).into(),
        }
    }
}

impl PulseChannel {
    pub fn reset(&mut self) {
        *self = Self {
            enabled: Enabled::disabled(),
            volume_and_envelope: VolumeAndEnvelope(0),
            length_enabled: false,
            period: (0).into(),
        };
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::VolumeAndEnvelope => self.volume_and_envelope.0,
            Register::PeriodLow => 0xff,
            Register::PeriodHighAndControl => PeriodHighAndControl::read(self.length_enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::VolumeAndEnvelope => self.volume_and_envelope = VolumeAndEnvelope(value),
            Register::PeriodLow => self.period.set_low8(value),
            Register::PeriodHighAndControl => {
                let value = PeriodHighAndControl(value);
                self.period.set_high3(value.period_high());
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
