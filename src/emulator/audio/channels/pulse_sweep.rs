use crate::emulator::audio::channels::{
    Enabled,
    registers::{PeriodHighAndControl, Signed11, VolumeAndEnvelope},
};

#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    VolumeAndEnvelope,
    Sweep,
    PeriodLow,
    PeriodHighAndControl,
}

pub struct PulseSweepChannel {
    pub enabled: Enabled,
    pub sweep: Sweep,
    pub volume_and_envelope: VolumeAndEnvelope,
    pub length_enabled: bool,
    pub period: Signed11,
}

impl Default for PulseSweepChannel {
    fn default() -> Self {
        Self {
            enabled: Enabled {
                enabled: true,
                output_left: true,
                output_right: true,
            },
            sweep: Sweep(0x80),
            volume_and_envelope: VolumeAndEnvelope(0),
            length_enabled: false,
            period: (-1).into(),
        }
    }
}

impl PulseSweepChannel {
    pub fn reset(&mut self) {
        *self = Self {
            enabled: Enabled::disabled(),
            sweep: Sweep(0),
            volume_and_envelope: VolumeAndEnvelope(0),
            length_enabled: false,
            period: (0).into(),
        }
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::VolumeAndEnvelope => self.volume_and_envelope.0,
            Register::Sweep => self.sweep.0,
            Register::PeriodLow => 0xff,
            Register::PeriodHighAndControl => PeriodHighAndControl::read(self.length_enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::VolumeAndEnvelope => self.volume_and_envelope = VolumeAndEnvelope(value),
            Register::Sweep => self.sweep.0 = value,
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

#[allow(dead_code)]
pub enum SweepDirection {
    Increasing,
    Decreasing,
}

pub struct Sweep(pub u8);
#[allow(dead_code)]
impl Sweep {
    pub fn pace(self) -> u8 {
        (self.0 & 0b0111_0000) >> 4
    }

    pub fn direction(self) -> SweepDirection {
        if self.0 & 0b1000 != 0 {
            SweepDirection::Increasing
        } else {
            SweepDirection::Decreasing
        }
    }

    pub fn step(self) -> u8 {
        self.0 & 0b111
    }
}
