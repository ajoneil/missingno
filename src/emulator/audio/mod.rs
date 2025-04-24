use crate::emulator::cpu::cycles::Cycles;
use channels::{
    Channels,
    noise::{self, NoiseChannel},
    pulse::{self, PulseChannel},
    pulse_sweep::{self, PulseSweepChannel},
    wave::{self, WaveChannel},
};
use volume::Volume;

pub mod channels;
pub mod length_timer;
pub mod period_sweep;
pub mod registers;
pub mod volume;
pub mod waveforms;

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

const SAMPLES_PER_FRAME: usize = 17556;

pub struct OutputBuffers {
    pub ch1: dasp_ring_buffer::Bounded<Vec<f32>>,
}

pub struct Audio {
    enabled: bool,
    channels: Channels,
    // volume_left: Volume,
    // volume_right: Volume,
    audio_timer_tick_in: Option<Cycles>,
    buffers: OutputBuffers,
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
            buffers: OutputBuffers {
                ch1: dasp_ring_buffer::Bounded::from(vec![0.0f32; SAMPLES_PER_FRAME]),
            },
            // volume_left: Volume::max(),
            // volume_right: Volume::max(),
            audio_timer_tick_in: None,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn channels(&self) -> &Channels {
        &self.channels
    }

    pub fn buffers(&self) -> &OutputBuffers {
        &self.buffers
    }

    // pub fn volume_left(&self) -> Volume {
    //     self.volume_left
    // }

    // pub fn volume_right(&self) -> Volume {
    //     self.volume_right
    // }

    pub fn trigger_audio_timer(&mut self, cycles: Cycles) {
        self.audio_timer_tick_in = Some(cycles);
    }

    pub fn step(&mut self, cycles: Cycles) {
        for _ in 0..cycles.0 {
            let audio_timer_tick = match self.audio_timer_tick_in {
                Some(tick) => {
                    if tick.0 == 0 {
                        self.audio_timer_tick_in = None;
                        true
                    } else {
                        self.audio_timer_tick_in = Some(tick - Cycles(1));
                        false
                    }
                }
                None => false,
            };

            if self.enabled() {
                let ch1_data = self.channels.ch1.step(audio_timer_tick);

                self.buffers.ch1.push(if self.channels.ch1.dac_enabled() {
                    (ch1_data as f32) / 0xf as f32
                } else {
                    0.0
                });
            } else {
                self.buffers.ch1.push(0.0);
            }
        }
    }
}
