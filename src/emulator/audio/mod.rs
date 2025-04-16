use channels::{
    Channels,
    noise::{self, NoiseChannel},
    pulse::{self, PulseChannel},
    pulse_sweep::{self, PulseSweepChannel},
    wave::{self, WaveChannel},
};
use volume::Volume;

pub mod channels;
pub mod registers;
pub mod volume;

#[derive(PartialEq, Eq, Debug)]
pub enum Register {
    Control,
    Panning,
    Volume,
    Channel1(pulse_sweep::Register),
    Channel2(pulse::Register),
    Channel3(wave::Register),
    Channel4(noise::Register),
}

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
