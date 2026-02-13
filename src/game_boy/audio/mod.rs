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

const SAMPLE_RATE: f32 = 44100.0;
const M_CYCLES_PER_SECOND: f32 = 1_048_576.0;
const M_CYCLES_PER_SAMPLE: f32 = M_CYCLES_PER_SECOND / SAMPLE_RATE;
const DIV_APU_BIT: u16 = 1 << 12; // Bit 12 of system counter drives frame sequencer

#[derive(Clone, nanoserde::SerRon, nanoserde::DeRon)]
pub struct Audio {
    pub enabled: bool,
    pub channels: Channels,
    pub volume_left: Volume,
    pub volume_right: Volume,
    pub nr50: u8,

    pub prev_div_apu_bit: bool,
    pub frame_sequencer_step: u8,
    #[nserde(skip)]
    sample_counter: f32,
    #[nserde(skip)]
    sample_buffer: Vec<(f32, f32)>,
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
            nr50: 0x77,

            prev_div_apu_bit: true, // matches initial internal_counter (0xABCC) bit 12
            frame_sequencer_step: 0,
            sample_counter: 0.0,
            sample_buffer: Vec::new(),
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

    pub fn mcycle(&mut self, div_counter: u16) {
        if !self.enabled {
            // Still track DIV-APU bit even when disabled, so we have the
            // correct previous state when APU is re-enabled.
            self.prev_div_apu_bit = div_counter & DIV_APU_BIT != 0;
            return;
        }

        // Advance channel frequency timers (4 T-cycles per M-cycle)
        for t in 0..4u8 {
            self.channels.ch1.tcycle();
            self.channels.ch2.tcycle();
            self.channels.ch3.tcycle(t);
            self.channels.ch4.tcycle();
        }

        // Frame sequencer: driven by falling edge of bit 12 in system counter (DIV-APU)
        let div_apu_bit = div_counter & DIV_APU_BIT != 0;
        if self.prev_div_apu_bit && !div_apu_bit {
            self.tick_frame_sequencer();
        }
        self.prev_div_apu_bit = div_apu_bit;

        // Downsample to output rate
        self.sample_counter += 1.0;
        if self.sample_counter >= M_CYCLES_PER_SAMPLE {
            self.sample_counter -= M_CYCLES_PER_SAMPLE;
            let sample = self.mix();
            self.sample_buffer.push(sample);
        }
    }

    fn tick_frame_sequencer(&mut self) {
        match self.frame_sequencer_step {
            0 | 4 => {
                self.channels.ch1.tick_length();
                self.channels.ch2.tick_length();
                self.channels.ch3.tick_length();
                self.channels.ch4.tick_length();
            }
            2 | 6 => {
                self.channels.ch1.tick_length();
                self.channels.ch2.tick_length();
                self.channels.ch3.tick_length();
                self.channels.ch4.tick_length();
                self.channels.ch1.tick_sweep();
            }
            7 => {
                self.channels.ch1.tick_envelope();
                self.channels.ch2.tick_envelope();
                self.channels.ch4.tick_envelope();
            }
            _ => {}
        }

        self.frame_sequencer_step = (self.frame_sequencer_step + 1) % 8;
    }

    /// Called when DIV is written (resetting internal counter to 0).
    /// If bit 12 was previously set, this is a falling edge → tick the frame sequencer.
    pub fn on_div_write(&mut self, old_counter: u16) {
        if self.enabled && old_counter & DIV_APU_BIT != 0 {
            self.tick_frame_sequencer();
        }
        self.prev_div_apu_bit = false; // counter is now 0, bit 12 is clear
    }

    fn mix(&self) -> (f32, f32) {
        let mut left = 0.0f32;
        let mut right = 0.0f32;

        let ch1 = self.channels.ch1.sample();
        let ch2 = self.channels.ch2.sample();
        let ch3 = self.channels.ch3.sample();
        let ch4 = self.channels.ch4.sample();

        if self.channels.ch1.enabled.output_left {
            left += ch1;
        }
        if self.channels.ch1.enabled.output_right {
            right += ch1;
        }
        if self.channels.ch2.enabled.output_left {
            left += ch2;
        }
        if self.channels.ch2.enabled.output_right {
            right += ch2;
        }
        if self.channels.ch3.enabled.output_left {
            left += ch3;
        }
        if self.channels.ch3.enabled.output_right {
            right += ch3;
        }
        if self.channels.ch4.enabled.output_left {
            left += ch4;
        }
        if self.channels.ch4.enabled.output_right {
            right += ch4;
        }

        // Scale by master volume and normalize (4 channels max)
        left = left / 4.0 * self.volume_left.percentage();
        right = right / 4.0 * self.volume_right.percentage();

        (left, right)
    }

    pub fn drain_samples(&mut self) -> Vec<(f32, f32)> {
        std::mem::take(&mut self.sample_buffer)
    }
}
