use crate::game_boy::audio::channels::{
    Enabled,
    registers::{
        EnvelopeDirection, LengthTimerAndDuty, PeriodHighAndControl, Signed11, VolumeAndEnvelope,
    },
};

const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
    [0, 0, 0, 0, 0, 0, 1, 1], // 25%
    [0, 0, 0, 0, 1, 1, 1, 1], // 50%
    [1, 1, 1, 1, 1, 1, 0, 0], // 75%
];

#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    LengthTimerAndDuty,
    VolumeAndEnvelope,
    PeriodLow,
    PeriodHighAndControl,
}

pub struct PulseChannel {
    pub enabled: Enabled,
    pub length_timer_and_duty: LengthTimerAndDuty,
    pub volume_and_envelope: VolumeAndEnvelope,
    pub length_enabled: bool,
    pub period: Signed11,

    frequency_timer: u16,
    wave_duty_position: u8,
    current_volume: u8,
    envelope_timer: u8,
    length_counter: u16,
}

impl Default for PulseChannel {
    fn default() -> Self {
        Self {
            enabled: Enabled {
                enabled: false,
                output_left: true,
                output_right: true,
            },
            length_timer_and_duty: LengthTimerAndDuty(0x3f),
            volume_and_envelope: VolumeAndEnvelope(0xf3),
            length_enabled: false,
            period: (-1).into(),

            frequency_timer: 0,
            wave_duty_position: 0,
            current_volume: 0,
            envelope_timer: 0,
            length_counter: 0,
        }
    }
}

impl PulseChannel {
    pub fn reset(&mut self) {
        *self = Self {
            enabled: Enabled::disabled(),
            length_timer_and_duty: LengthTimerAndDuty(0),
            volume_and_envelope: VolumeAndEnvelope(0),
            length_enabled: false,
            period: (0).into(),

            frequency_timer: 0,
            wave_duty_position: 0,
            current_volume: 0,
            envelope_timer: 0,
            length_counter: 0,
        };
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::LengthTimerAndDuty => self.length_timer_and_duty.0,
            Register::VolumeAndEnvelope => self.volume_and_envelope.0,
            Register::PeriodLow => 0xff,
            Register::PeriodHighAndControl => PeriodHighAndControl::read(self.length_enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::LengthTimerAndDuty => {
                self.length_timer_and_duty = LengthTimerAndDuty(value);
                self.length_counter = 64 - self.length_timer_and_duty.initial_length() as u16;
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

    pub fn trigger(&mut self) {
        self.enabled.enabled = true;
        if self.length_counter == 0 {
            self.length_counter = 64;
        }
        self.frequency_timer = (2048 - self.period.0) * 4;
        self.current_volume = self.volume_and_envelope.initial_volume();
        self.envelope_timer = self.volume_and_envelope.sweep_pace();

        // DAC check: if upper 5 bits of volume register are 0, channel is disabled
        if self.volume_and_envelope.0 & 0xf8 == 0 {
            self.enabled.enabled = false;
        }
    }

    pub fn tick(&mut self) {
        if self.frequency_timer > 0 {
            self.frequency_timer -= 1;
        }
        if self.frequency_timer == 0 {
            self.frequency_timer = (2048 - self.period.0) * 4;
            self.wave_duty_position = (self.wave_duty_position + 1) % 8;
        }
    }

    pub fn tick_length(&mut self) {
        if self.length_enabled && self.length_counter > 0 {
            self.length_counter -= 1;
            if self.length_counter == 0 {
                self.enabled.enabled = false;
            }
        }
    }

    pub fn tick_envelope(&mut self) {
        let pace = self.volume_and_envelope.sweep_pace();
        if pace == 0 {
            return;
        }

        if self.envelope_timer > 0 {
            self.envelope_timer -= 1;
        }
        if self.envelope_timer == 0 {
            self.envelope_timer = pace;
            match self.volume_and_envelope.direction() {
                EnvelopeDirection::Increase => {
                    if self.current_volume < 15 {
                        self.current_volume += 1;
                    }
                }
                EnvelopeDirection::Decrease => {
                    if self.current_volume > 0 {
                        self.current_volume -= 1;
                    }
                }
            }
        }
    }

    pub fn sample(&self) -> f32 {
        if !self.enabled.enabled {
            return 0.0;
        }
        let duty = self.length_timer_and_duty.wave_duty() as usize;
        let output = DUTY_TABLE[duty][self.wave_duty_position as usize];
        output as f32 * self.current_volume as f32 / 15.0
    }
}
