use crate::game_boy::audio::channels::{
    Enabled,
    registers::{EnvelopeDirection, VolumeAndEnvelope},
};

#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    LengthTimer,
    VolumeAndEnvelope,
    FrequencyAndRandomness,
    Control,
}

#[derive(Clone, nanoserde::SerRon, nanoserde::DeRon)]
pub struct NoiseChannel {
    pub enabled: Enabled,
    pub volume_and_envelope: VolumeAndEnvelope,
    pub length_enabled: bool,
    pub frequency_and_randomness: FrequencyAndRandomness,

    pub frequency_timer: u16,
    pub lfsr: u16,
    pub current_volume: u8,
    pub envelope_timer: u8,
    pub length_counter: u16,
}

impl Default for NoiseChannel {
    fn default() -> Self {
        Self {
            enabled: Enabled {
                enabled: false,
                output_left: true,
                output_right: false,
            },
            volume_and_envelope: VolumeAndEnvelope(0),
            length_enabled: false,
            frequency_and_randomness: FrequencyAndRandomness(0),

            frequency_timer: 0,
            lfsr: 0x7fff,
            current_volume: 0,
            envelope_timer: 0,
            length_counter: 0,
        }
    }
}

impl NoiseChannel {
    pub fn reset(&mut self) {
        self.enabled = Enabled::disabled();
        self.volume_and_envelope = VolumeAndEnvelope(0);
        self.length_enabled = false;
        self.frequency_and_randomness = FrequencyAndRandomness(0);

        self.frequency_timer = 0;
        self.lfsr = 0x7fff;
        self.current_volume = 0;
        self.envelope_timer = 0;
        self.length_counter = 0;
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::LengthTimer => 0xff,
            Register::VolumeAndEnvelope => self.volume_and_envelope.0,
            Register::FrequencyAndRandomness => self.frequency_and_randomness.0,
            Register::Control => Control::read(self.length_enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::LengthTimer => {
                self.length_counter = 64 - (value & 0x3f) as u16;
            }
            Register::VolumeAndEnvelope => self.volume_and_envelope = VolumeAndEnvelope(value),
            Register::FrequencyAndRandomness => {
                self.frequency_and_randomness = FrequencyAndRandomness(value)
            }
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
        self.enabled.enabled = true;
        if self.length_counter == 0 {
            self.length_counter = 64;
        }
        self.frequency_timer = self.frequency_and_randomness.timer_period();
        self.lfsr = 0x7fff;
        self.current_volume = self.volume_and_envelope.initial_volume();
        self.envelope_timer = self.volume_and_envelope.sweep_pace();

        // DAC check
        if self.volume_and_envelope.0 & 0xf8 == 0 {
            self.enabled.enabled = false;
        }
    }

    pub fn tcycle(&mut self) {
        if self.frequency_timer > 0 {
            self.frequency_timer -= 1;
        }
        if self.frequency_timer == 0 {
            self.frequency_timer = self.frequency_and_randomness.timer_period();

            // Clock LFSR
            let xor_result = (self.lfsr & 1) ^ ((self.lfsr >> 1) & 1);
            self.lfsr >>= 1;
            self.lfsr |= xor_result << 14;

            // 7-bit width mode
            if self.frequency_and_randomness.short_mode() {
                self.lfsr &= !(1 << 6);
                self.lfsr |= xor_result << 6;
            }
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
        // Output is inverted bit 0 of LFSR
        let output = (!self.lfsr & 1) as f32;
        output * self.current_volume as f32 / 15.0
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

#[derive(Clone, nanoserde::SerRon, nanoserde::DeRon)]
pub struct FrequencyAndRandomness(pub u8);

impl FrequencyAndRandomness {
    pub fn clock_shift(&self) -> u8 {
        self.0 >> 4
    }

    pub fn short_mode(&self) -> bool {
        self.0 & 0b1000 != 0
    }

    pub fn divisor_code(&self) -> u8 {
        self.0 & 0b111
    }

    fn timer_period(&self) -> u16 {
        let divisor = match self.divisor_code() {
            0 => 8,
            n => (n as u16) * 16,
        };
        divisor << self.clock_shift()
    }
}
