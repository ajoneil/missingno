use crate::emulator::audio::{
    channels::{Enabled, registers::PeriodHighAndControl},
    length_timer::LengthTimer,
    period_sweep::PeriodSweep,
    volume::{EnvelopeDirection, Volume},
    waveforms::Waveform,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    WaveformAndInitialLength,
    Volume,
    PeriodSweep,
    PeriodLow,
    PeriodHighAndControl,
}

pub struct PulseSweepChannel {
    pub enabled: Enabled,
    period_sweep: PeriodSweep,
    length_timer: LengthTimer,
    waveform: Waveform,

    volume: Volume,

    pub period: u16,
    current_period: u16,
}

impl Default for PulseSweepChannel {
    fn default() -> Self {
        Self {
            enabled: Enabled {
                enabled: false,
                output_left: true,
                output_right: true,
            },
            period_sweep: PeriodSweep::new(0x80),
            length_timer: LengthTimer::new(),
            volume: Volume::new(0xf, EnvelopeDirection::Decrease, 3),
            period: 0x7ff,
            current_period: 0x7ff,
            waveform: Waveform::new(2),
        }
    }
}

impl PulseSweepChannel {
    pub fn reset(&mut self) {
        *self = Self {
            enabled: Enabled::disabled(),
            period_sweep: PeriodSweep::new(0),
            length_timer: LengthTimer::new(),
            waveform: Waveform::new(2),
            volume: Volume::new(0, EnvelopeDirection::Decrease, 0),
            period: 0,
            current_period: 0,
        }
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::WaveformAndInitialLength => self.waveform.waveform() << 6 | 0x3f,
            Register::Volume => self.volume.read_register(),
            Register::PeriodSweep => self.period_sweep.read_register(),
            Register::PeriodLow => 0xff,
            Register::PeriodHighAndControl => {
                PeriodHighAndControl::read(self.length_timer.enabled())
            }
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::WaveformAndInitialLength => {
                self.waveform.set_waveform(value >> 6);
                self.length_timer.set_initial_length(value & 0x3f);
            }

            Register::Volume => {
                self.volume.write_register(value);
                if !self.dac_enabled() {
                    self.enabled.enabled = false;
                }
            }
            Register::PeriodSweep => self.period_sweep.write_register(value),
            Register::PeriodLow => self.period = (self.period ^ 0xff) | value as u16,
            Register::PeriodHighAndControl => {
                let value = PeriodHighAndControl(value);
                self.period = (self.period & 0xff) | ((value.period_high() as u16) << 8);

                if value.enable_length() {
                    self.length_timer.enable();
                } else {
                    self.length_timer.disable();
                }

                if value.trigger() {
                    self.trigger();
                }
            }
        }
    }

    pub fn dac_enabled(&self) -> bool {
        self.volume.initial_volume() > 0 || self.volume.direction() == EnvelopeDirection::Increase
    }

    pub fn trigger(&mut self) {
        self.enabled.enabled = true;
        self.current_period = self.period;
        self.volume.trigger();
        self.period_sweep.trigger(self.period);
        self.waveform.trigger(self.period);
        self.length_timer.trigger();
    }

    pub fn step(&mut self, audio_timer_tick: bool) -> u8 {
        if self.enabled.enabled {
            if audio_timer_tick {
                self.volume.tick();
                if let Some(new_period) = self.period_sweep.tick(self.current_period) {
                    self.current_period = new_period;
                }

                if self.length_timer.tick() {
                    self.enabled.enabled = false;
                }
            }

            self.waveform
                .tick(self.current_period, self.volume.current_volume())
        } else {
            0
        }
    }
}
