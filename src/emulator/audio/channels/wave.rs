use crate::emulator::audio::{
    Audio,
    channels::{
        Enabled,
        registers::{PeriodHighAndControl, Signed11},
    },
};

#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    Volume,
    DacEnabled,
    Length,
    PeriodLow,
    PeriodHighAndControl,
}

pub struct WaveChannel {
    pub enabled: Enabled,
    pub dac_enabled: bool,
    pub volume: Volume,
    pub length: u8,
    pub length_enabled: bool,
    pub period: Signed11,
    pub ram: [u8; 16],
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
            length: 0xff,
            length_enabled: false,
            period: (-1).into(),
            ram: [0; 16],
        }
    }
}

impl WaveChannel {
    pub fn reset(&mut self) {
        *self = Self {
            enabled: Enabled::disabled(),
            dac_enabled: false,
            volume: Volume(0),
            length: 0,
            length_enabled: false,
            period: 0.into(),
            ram: [0; 16],
        };
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Volume => self.volume.0,
            Register::Length => self.length,
            Register::DacEnabled => {
                if self.dac_enabled {
                    0xff
                } else {
                    0x7f
                }
            }
            Register::PeriodLow => 0xff,
            Register::PeriodHighAndControl => PeriodHighAndControl::read(self.length_enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::Volume => self.volume = Volume(value),
            Register::Length => self.length = value,
            Register::DacEnabled => {
                self.dac_enabled = value & 0b1000_0000 != 0;
            }
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
        // TODO audio
    }
}

pub struct Volume(pub u8);
impl Volume {
    pub fn volume(&self) -> f32 {
        ((self.0 >> 5) & 0b11) as f32 / 4.0
    }
}

impl Audio {
    pub fn read_wave_ram(&self, offset: u8) -> u8 {
        self.channels.ch3.ram[offset as usize]
    }

    pub fn write_wave_ram(&mut self, offset: u8, value: u8) {
        self.channels.ch3.ram[offset as usize] = value;
    }
}
