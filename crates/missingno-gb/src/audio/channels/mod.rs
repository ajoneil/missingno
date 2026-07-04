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
        self.ch1.tick_envelope_counter();
        self.ch2.tick_envelope_counter();
        self.ch4.tick_envelope_counter();
    }

    pub fn reset_all(&mut self) {
        self.ch1.reset();
        self.ch2.reset();
        self.ch3.reset();
        self.ch4.reset();
    }

    /// Drain the four channels' `output_dirty` flags; true when any
    /// `mix_digital()` input may have changed since the last drain.
    pub fn take_output_dirty(&mut self) -> bool {
        let dirty = self.ch1.output_dirty
            | self.ch2.output_dirty
            | self.ch3.output_dirty
            | self.ch4.output_dirty;
        self.ch1.output_dirty = false;
        self.ch2.output_dirty = false;
        self.ch3.output_dirty = false;
        self.ch4.output_dirty = false;
        dirty
    }

    /// Sum the four channels' digital outputs (0–15 each) into a
    /// `(left, right)` pair, gated by each channel's panning bits.
    pub fn mix_digital(&self) -> (u32, u32) {
        let mut left = 0u32;
        let mut right = 0u32;
        let mut mix = |enabled: Enabled, sample: u8| {
            if enabled.output_left {
                left += sample as u32;
            }
            if enabled.output_right {
                right += sample as u32;
            }
        };
        mix(self.ch1.enabled, self.ch1.digital_sample());
        mix(self.ch2.enabled, self.ch2.digital_sample());
        mix(self.ch3.enabled, self.ch3.digital_sample());
        mix(self.ch4.enabled, self.ch4.digital_sample());
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
