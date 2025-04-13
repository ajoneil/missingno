pub mod channels;
pub mod registers;
pub mod volume;

use channels::{
    Channels, noise::NoiseChannel, pulse::PulseChannel, pulse_sweep::PulseSweepChannel,
    wave::WaveChannel,
};
pub use registers::Register;
use volume::Volume;

pub struct Audio {
    enabled: bool,
    channels: Channels,
    volume_left: Volume,
    volume_right: Volume,
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
            volume_left: Volume::max(),
            volume_right: Volume::max(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn channels(&self) -> &Channels {
        &self.channels
    }

    pub fn volume_left(&self) -> Volume {
        self.volume_left
    }

    pub fn volume_right(&self) -> Volume {
        self.volume_right
    }
}
