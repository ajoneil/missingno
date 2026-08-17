use noise::NoiseChannel;
use pulse::PulseChannel;
use sweep::{NoSweep, Sweep};
use wave::WaveChannel;

pub mod envelope;
pub mod length;
pub mod noise;
pub mod pulse;
pub mod registers;
pub mod sweep;
pub mod wave;

/// A trigger-armed divider reload pending on CH1/CH2, latched at the NRx4
/// write and consumed at the next chN_1mhz↑ (the chN_restart sync). The
/// enabling case (fdis 1→0) freezes the load tick for the +1 first overflow;
/// a re-trigger of a running channel reloads with no +1.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerReload {
    #[default]
    Idle,
    Retrigger,
    Enabling,
}

#[derive(Clone, Default)]
pub struct Channels {
    pub ch1: PulseChannel<Sweep>,
    pub ch2: PulseChannel<NoSweep>,
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

    pub fn reset_all(&mut self) {
        self.ch1.reset();
        self.ch2.reset();
        self.ch3.reset();
        self.ch4.reset();
    }

    /// Drain the four channels' `output_dirty` flags; true when any
    /// `mix_dac()` input may have changed since the last drain.
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

    /// Sum the four DAC outputs into a `(left, right)` pair, gated by each
    /// channel's panning bits, in half-LSB units: a powered DAC drives
    /// `15 - 2*digital` (digital 0 = +15, the positive extreme through the
    /// inverting volume amp); an unpowered DAC is high-impedance and
    /// contributes nothing.
    pub fn mix_dac(&self) -> (i32, i32) {
        let mut left = 0i32;
        let mut right = 0i32;
        let mut mix = |enabled: Enabled, dac_enabled: bool, sample: u8| {
            if !dac_enabled {
                return;
            }
            let level = 15 - 2 * sample as i32;
            if enabled.output_left {
                left += level;
            }
            if enabled.output_right {
                right += level;
            }
        };
        mix(
            self.ch1.enabled,
            self.ch1.dac_enabled(),
            self.ch1.digital_sample(),
        );
        mix(
            self.ch2.enabled,
            self.ch2.dac_enabled(),
            self.ch2.digital_sample(),
        );
        mix(
            self.ch3.enabled,
            self.ch3.dac_enabled(),
            self.ch3.digital_sample(),
        );
        mix(
            self.ch4.enabled,
            self.ch4.dac_enabled(),
            self.ch4.digital_sample(),
        );
        (left, right)
    }

    /// Each channel's DAC input code (0-15), or 0 when its DAC is unpowered —
    /// the code the silicon hands the DAC, for the debugger's waveform capture.
    pub fn dac_codes(&self) -> [u8; 4] {
        let code = |dac_on: bool, sample: u8| if dac_on { sample } else { 0 };
        [
            code(self.ch1.dac_enabled(), self.ch1.digital_sample()),
            code(self.ch2.dac_enabled(), self.ch2.digital_sample()),
            code(self.ch3.dac_enabled(), self.ch3.digital_sample()),
            code(self.ch4.dac_enabled(), self.ch4.digital_sample()),
        ]
    }

    /// Whether each channel's DAC is currently powered.
    pub fn dac_active(&self) -> [bool; 4] {
        [
            self.ch1.dac_enabled(),
            self.ch2.dac_enabled(),
            self.ch3.dac_enabled(),
            self.ch4.dac_enabled(),
        ]
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
