pub mod noise;
pub mod pulse;
pub mod pulse_sweep;
pub mod wave;

use noise::NoiseChannel;
use pulse::PulseChannel;
use pulse_sweep::PulseSweepChannel;
use wave::WaveChannel;

#[derive(PartialEq, Eq, Debug)]
pub enum Channel {
    Channel1,
    Channel2,
    Channel3,
    Channel4,
}

pub struct Channels {
    pub ch1: PulseSweepChannel,
    pub ch2: PulseChannel,
    pub ch3: WaveChannel,
    pub ch4: NoiseChannel,
}

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
