use crate::emulator::audio::channels::{
    Enabled,
    registers::{
        EnvelopeDirection, PeriodHighAndControl, Signed11, VolumeAndEnvelope,
        WaveformAndInitialLength,
    },
};

#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    WaveformAndInitialLength,
    VolumeAndEnvelope,
    PeriodLow,
    PeriodHighAndControl,
}

pub struct PulseChannel {
    pub enabled: Enabled,
    pub length_timer_and_duty: WaveformAndInitialLength,
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
            length_timer_and_duty: WaveformAndInitialLength(0x3f),
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
            length_timer_and_duty: WaveformAndInitialLength(0),
            volume_and_envelope: VolumeAndEnvelope(0),
            length_enabled: false,
            period: (0).into(),
        };
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::WaveformAndInitialLength => self.length_timer_and_duty.0,
            Register::VolumeAndEnvelope => self.volume_and_envelope.0,
            Register::PeriodLow => 0xff,
            Register::PeriodHighAndControl => PeriodHighAndControl::read(self.length_enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::WaveformAndInitialLength => {
                self.length_timer_and_duty = WaveformAndInitialLength(value)
            }
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

    pub fn dac_enabled(&self) -> bool {
        self.volume_and_envelope.initial_volume() > 0
            || self.volume_and_envelope.direction() == EnvelopeDirection::Increase
    }

    pub fn trigger(&mut self) {
        // TODO audio
    }
}
