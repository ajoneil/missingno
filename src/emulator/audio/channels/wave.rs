use crate::emulator::audio::channels::Enabled;

#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    Volume,
    DacEnabled,
}

pub struct WaveChannel {
    pub enabled: Enabled,
    pub dac_enabled: bool,
    pub volume: Volume,
}

impl Default for WaveChannel {
    fn default() -> Self {
        Self {
            enabled: Enabled {
                enabled: false,
                output_left: true,
                output_right: false,
            },
            dac_enabled: false,
            volume: Volume(0x9f),
        }
    }
}

impl WaveChannel {
    pub fn reset(&mut self) {
        *self = Self {
            enabled: Enabled::disabled(),
            dac_enabled: false,
            volume: Volume(0),
        };
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Volume => self.volume.0,
            Register::DacEnabled => {
                if self.dac_enabled {
                    0xff
                } else {
                    0x7f
                }
            }
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::Volume => self.volume = Volume(value),
            Register::DacEnabled => {
                self.dac_enabled = value & 0b1000_0000 != 0;
            }
        }
    }
}

pub struct Volume(pub u8);
impl Volume {
    pub fn volume(&self) -> f32 {
        ((self.0 >> 5) & 0b11) as f32 / 4.0
    }
}
