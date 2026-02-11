use crate::game_boy::audio::channels::{
    Enabled,
    registers::{
        EnvelopeDirection, PeriodHighAndControl, Signed11, VolumeAndEnvelope,
        WaveformAndInitialLength,
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
    WaveformAndInitialLength,
    Volume,
    PeriodSweep,
    PeriodLow,
    PeriodHighAndControl,
}

#[derive(Clone, nanoserde::SerRon, nanoserde::DeRon)]
pub struct PulseSweepChannel {
    pub enabled: Enabled,
    pub sweep: Sweep,
    pub waveform_and_initial_length: WaveformAndInitialLength,
    pub volume_and_envelope: VolumeAndEnvelope,
    pub length_enabled: bool,
    pub period: Signed11,

    pub frequency_timer: u16,
    pub wave_duty_position: u8,
    pub current_volume: u8,
    pub envelope_timer: u8,
    pub length_counter: u16,
    pub shadow_frequency: u16,
    pub sweep_timer: u8,
    pub sweep_enabled: bool,
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
            waveform_and_initial_length: WaveformAndInitialLength(0xbf),
            volume_and_envelope: VolumeAndEnvelope(0xf3),
            length_enabled: false,
            period: (-1).into(),

            frequency_timer: 0,
            wave_duty_position: 0,
            current_volume: 0,
            envelope_timer: 0,
            length_counter: 0,
            shadow_frequency: 0,
            sweep_timer: 0,
            sweep_enabled: false,
        }
    }
}

impl PulseSweepChannel {
    pub fn reset(&mut self) {
        *self = Self {
            enabled: Enabled::disabled(),
            sweep: Sweep(0),
            waveform_and_initial_length: WaveformAndInitialLength(0),
            volume_and_envelope: VolumeAndEnvelope(0),
            length_enabled: false,
            period: (0).into(),

            frequency_timer: 0,
            wave_duty_position: 0,
            current_volume: 0,
            envelope_timer: 0,
            length_counter: 0,
            shadow_frequency: 0,
            sweep_timer: 0,
            sweep_enabled: false,
        }
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::WaveformAndInitialLength => self.waveform_and_initial_length.0 | 0x3F,
            Register::Volume => self.volume_and_envelope.0,
            Register::PeriodSweep => self.sweep.0 | 0x80,
            Register::PeriodLow => 0xff,
            Register::PeriodHighAndControl => PeriodHighAndControl::read(self.length_enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::WaveformAndInitialLength => {
                self.waveform_and_initial_length = WaveformAndInitialLength(value);
                self.length_counter = 64 - self.waveform_and_initial_length.initial_length() as u16;
            }
            Register::Volume => self.volume_and_envelope = VolumeAndEnvelope(value),
            Register::PeriodSweep => self.sweep.0 = value,
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

        // Initialize sweep
        self.shadow_frequency = self.period.0;
        let pace = self.sweep.pace();
        self.sweep_timer = if pace != 0 { pace } else { 8 };
        self.sweep_enabled = pace != 0 || self.sweep.step() != 0;

        // If step is non-zero, do overflow check
        if self.sweep.step() != 0 && self.calculate_sweep_frequency() > 2047 {
            self.enabled.enabled = false;
        }

        // DAC check
        if self.volume_and_envelope.0 & 0xf8 == 0 {
            self.enabled.enabled = false;
        }
    }

    fn calculate_sweep_frequency(&self) -> u16 {
        let shifted = self.shadow_frequency >> self.sweep.step();
        match self.sweep.direction() {
            SweepDirection::Increasing => self.shadow_frequency.wrapping_add(shifted),
            SweepDirection::Decreasing => self.shadow_frequency.wrapping_sub(shifted),
        }
    }

    pub fn tcycle(&mut self) {
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

    pub fn tick_sweep(&mut self) {
        if !self.sweep_enabled {
            return;
        }

        if self.sweep_timer > 0 {
            self.sweep_timer -= 1;
        }
        if self.sweep_timer == 0 {
            let pace = self.sweep.pace();
            self.sweep_timer = if pace != 0 { pace } else { 8 };

            if pace != 0 {
                let new_frequency = self.calculate_sweep_frequency();
                if new_frequency > 2047 {
                    self.enabled.enabled = false;
                } else if self.sweep.step() != 0 {
                    self.shadow_frequency = new_frequency;
                    self.period.0 = new_frequency;

                    // Overflow check again with new frequency
                    if self.calculate_sweep_frequency() > 2047 {
                        self.enabled.enabled = false;
                    }
                }
            }
        }
    }

    pub fn sample(&self) -> f32 {
        if !self.enabled.enabled {
            return 0.0;
        }
        let duty = self.waveform_and_initial_length.waveform() as usize;
        let output = DUTY_TABLE[duty][self.wave_duty_position as usize];
        output as f32 * self.current_volume as f32 / 15.0
    }
}

pub enum SweepDirection {
    Increasing,
    Decreasing,
}

#[derive(Clone, nanoserde::SerRon, nanoserde::DeRon)]
pub struct Sweep(pub u8);

impl Sweep {
    pub fn pace(&self) -> u8 {
        (self.0 & 0b0111_0000) >> 4
    }

    pub fn direction(&self) -> SweepDirection {
        if self.0 & 0b1000 != 0 {
            SweepDirection::Decreasing
        } else {
            SweepDirection::Increasing
        }
    }

    pub fn step(&self) -> u8 {
        self.0 & 0b111
    }
}
