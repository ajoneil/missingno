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
const DIV_APU_BIT: u16 = 1 << 10; // Bit 10 of M-cycle counter drives frame sequencer

#[derive(Clone)]
pub struct Audio {
    pub enabled: bool,
    pub channels: Channels,
    pub volume_left: Volume,
    pub volume_right: Volume,
    pub nr50: u8,

    pub prev_div_apu_bit: bool,
    pub frame_sequencer_step: u8,
    sample_counter: f32,
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

            prev_div_apu_bit: false, // matches initial internal_counter (0x2AF3) bit 10
            frame_sequencer_step: 0,
            sample_counter: 0.0,
            sample_buffer: Vec::new(),
        }
    }

    /// Power-on state: audio disabled, all registers zeroed.
    pub fn power_on() -> Self {
        Self {
            enabled: false,
            channels: Channels {
                ch1: PulseSweepChannel::default(),
                ch2: PulseChannel::default(),
                ch3: WaveChannel::default(),
                ch4: NoiseChannel::default(),
            },
            volume_left: Volume(0),
            volume_right: Volume(0),
            nr50: 0x00,
            prev_div_apu_bit: false, // internal_counter starts at 0, bit 12 = 0
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
        self.prev_div_apu_bit = false; // counter is now 0, bit 10 is clear
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

    /// Construct an Audio instance from a gbtrace snapshot.
    #[cfg(feature = "gbtrace")]
    pub fn from_snapshot(snap: &gbtrace::snapshot::ApuSnapshot, wave_ram: [u8; 16]) -> Self {
        use channels::{Enabled, pulse::PulseChannel, pulse_sweep::{PulseSweepChannel, Sweep}, wave::WaveChannel, noise::NoiseChannel};
        use channels::registers::{Signed11, VolumeAndEnvelope, WaveformAndInitialLength};
        use channels::noise::FrequencyAndRandomness;
        use channels::wave::Volume as WaveVolume;

        let channels = Channels {
            ch1: PulseSweepChannel {
                enabled: Enabled { enabled: true, output_left: true, output_right: true },
                sweep: Sweep(snap.ch1_sweep),
                waveform_and_initial_length: WaveformAndInitialLength(snap.ch1_duty_len),
                volume_and_envelope: VolumeAndEnvelope(snap.ch1_vol_env),
                length_enabled: snap.ch1_length_enabled,
                period: Signed11(snap.ch1_period),
                frequency_timer: 0,
                wave_duty_position: 0,
                current_volume: 0,
                envelope_timer: snap.ch1_envelope_timer,
                length_counter: 0,
                shadow_frequency: snap.ch1_period,
                sweep_timer: snap.ch1_sweep_timer,
                sweep_enabled: snap.ch1_sweep_enabled,
                sweep_negate_used: snap.ch1_sweep_negate_used,
            },
            ch2: PulseChannel {
                enabled: Enabled { enabled: true, output_left: true, output_right: true },
                waveform_and_initial_length: WaveformAndInitialLength(snap.ch2_duty_len),
                volume_and_envelope: VolumeAndEnvelope(snap.ch2_vol_env),
                length_enabled: snap.ch2_length_enabled,
                period: Signed11(snap.ch2_period),
                frequency_timer: 0,
                wave_duty_position: 0,
                current_volume: 0,
                envelope_timer: snap.ch2_envelope_timer,
                length_counter: 0,
            },
            ch3: WaveChannel {
                enabled: Enabled { enabled: true, output_left: true, output_right: true },
                dac_enabled: snap.ch3_dac & 0x80 != 0,
                volume: WaveVolume(snap.ch3_vol),
                length_enabled: snap.ch3_length_enabled,
                period: Signed11(snap.ch3_period),
                ram: wave_ram,
                frequency_timer: 0,
                wave_position: 0,
                length_counter: 0,
                sample_read_tcycle: 0xFF,
            },
            ch4: NoiseChannel {
                enabled: Enabled { enabled: true, output_left: true, output_right: true },
                volume_and_envelope: VolumeAndEnvelope(snap.ch4_vol_env),
                length_enabled: snap.ch4_length_enabled,
                frequency_and_randomness: FrequencyAndRandomness(snap.ch4_freq),
                frequency_timer: 0,
                lfsr: 0x7FFF,
                current_volume: 0,
                envelope_timer: snap.ch4_envelope_timer,
                length_counter: 0,
            },
        };

        Self {
            enabled: snap.sound_on & 0x80 != 0,
            channels,
            volume_left: Volume(0),
            volume_right: Volume(0),
            nr50: snap.master_vol,
            prev_div_apu_bit: snap.prev_div_apu_bit,
            frame_sequencer_step: snap.frame_sequencer_step,
            sample_counter: 0.0,
            sample_buffer: Vec::new(),
        }
    }
}
