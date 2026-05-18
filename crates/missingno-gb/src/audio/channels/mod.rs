use noise::NoiseChannel;
use pulse::PulseChannel;
use pulse_sweep::PulseSweepChannel;
use wave::WaveChannel;

pub mod noise;
pub mod pulse;
pub mod pulse_sweep;
pub mod registers;
pub mod wave;

#[derive(Clone, Default)]
pub struct Channels {
    pub ch1: PulseSweepChannel,
    pub ch2: PulseChannel,
    pub ch3: WaveChannel,
    pub ch4: NoiseChannel,
}

impl Channels {
    pub fn tick_length_all(&mut self) {
        self.ch1.tick_length();
        self.ch2.tick_length();
        self.ch3.tick_length();
        self.ch4.tick_length();
    }

    pub fn tick_envelope_all(&mut self) {
        self.ch1.tick_envelope();
        self.ch2.tick_envelope();
        self.ch4.tick_envelope();
    }

    pub fn reset_all(&mut self) {
        self.ch1.reset();
        self.ch2.reset();
        self.ch3.reset();
        self.ch4.reset();
    }

    /// Mix all four channels into a `(left, right)` sample pair,
    /// gated by each channel's left/right panning bits.
    pub fn mix(&self) -> (f32, f32) {
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        for (enabled, sample) in [
            (self.ch1.enabled, self.ch1.sample()),
            (self.ch2.enabled, self.ch2.sample()),
            (self.ch3.enabled, self.ch3.sample()),
            (self.ch4.enabled, self.ch4.sample()),
        ] {
            if enabled.output_left {
                left += sample;
            }
            if enabled.output_right {
                right += sample;
            }
        }
        (left, right)
    }
}

#[derive(Copy, Clone)]
pub struct Enabled {
    pub enabled: bool,
    pub output_left: bool,
    pub output_right: bool,
}

impl Enabled {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            output_left: false,
            output_right: false,
        }
    }
}
