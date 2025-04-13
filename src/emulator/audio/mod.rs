pub mod channels;
pub mod registers;

use channels::{
    Channels, noise::NoiseChannel, pulse::PulseChannel, pulse_sweep::PulseSweepChannel,
    wave::WaveChannel,
};
pub use registers::Register;

pub struct Audio {
    enabled: bool,
    channels: Channels,
}

impl Audio {
    pub fn new() -> Self {
        Self {
            enabled: true,
            channels: Channels {
                ch1: PulseSweepChannel::default(),
                ch2: PulseChannel::default(),
                ch3: WaveChannel::default(),
                ch4: NoiseChannel::default(),
            },
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn channels(&self) -> &Channels {
        &self.channels
    }
}
