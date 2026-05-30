use channels::{Channels, noise, pulse, pulse_sweep, wave};
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
const T_CYCLES_PER_SECOND: f32 = 4_194_304.0;
const T_CYCLES_PER_SAMPLE: f32 = T_CYCLES_PER_SECOND / SAMPLE_RATE;
const DIV_APU_BIT: u16 = 1 << 10; // Bit 10 of M-cycle counter drives frame sequencer

#[derive(Clone)]
pub struct Audio {
    pub(crate) enabled: bool,
    pub(crate) channels: Channels,
    pub(crate) volume_left: Volume,
    pub(crate) volume_right: Volume,
    pub(crate) nr50: u8,

    pub(crate) prev_div_apu_bit: bool,
    pub(crate) frame_sequencer_step: u8,
    sample_counter: f32,
    sample_accum_left: f32,
    sample_accum_right: f32,
    sample_accum_count: u32,
    sample_buffer: Vec<(f32, f32)>,
}

impl Audio {
    /// Post-boot state at PC=0x0100. `internal_counter` is the M-cycle
    /// `reg_div16` (UKUP..UPOF) — fs_step and prev_div_apu_bit derive
    /// from it so the frame sequencer is in phase with hardware's
    /// kene/byfe_128hz divider chain at the §11.1 anchor.
    pub fn post_boot(internal_counter: u16) -> Self {
        Self {
            enabled: true,
            channels: Channels::default(),
            volume_left: Volume::max(),
            volume_right: Volume::max(),
            nr50: 0x77,

            prev_div_apu_bit: internal_counter & DIV_APU_BIT != 0,
            frame_sequencer_step: ((internal_counter >> 11) & 0x7) as u8,
            sample_counter: 0.0,
            sample_accum_left: 0.0,
            sample_accum_right: 0.0,
            sample_accum_count: 0,
            sample_buffer: Vec::new(),
        }
    }

    /// Power-on state: audio disabled, all registers zeroed.
    pub fn new() -> Self {
        Self {
            enabled: false,
            channels: Channels::default(),
            volume_left: Volume(0),
            volume_right: Volume(0),
            nr50: 0x00,
            prev_div_apu_bit: false, // internal_counter starts at 0, bit 12 = 0
            frame_sequencer_step: 0,
            sample_counter: 0.0,
            sample_accum_left: 0.0,
            sample_accum_right: 0.0,
            sample_accum_count: 0,
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

    pub fn nr50(&self) -> u8 {
        self.nr50
    }

    pub fn frame_sequencer_step(&self) -> u8 {
        self.frame_sequencer_step
    }

    /// `bufy_256hz` LOW = `caru` (ripple bit 0) low = `C` even — the
    /// deme NOR length-clock gate's level input that an NRx4 length-enable
    /// 0→1 write reads to decide the extra clock.
    fn caru_low(&self) -> bool {
        self.frame_sequencer_step % 2 == 0
    }

    pub fn prev_div_apu_bit(&self) -> bool {
        self.prev_div_apu_bit
    }

    /// One T-cycle of APU work, called at every master-clock rise.
    /// `apu_reset_n` is NR52 bit 7 — the channels' prescaler DFFs
    /// honour it as an async-reset, so we still call each tcycle
    /// unconditionally to keep the reset edge observable.
    pub fn tcycle(&mut self, div_counter: u16, t_index: u8) {
        let apu_reset_n = self.enabled;
        self.channels.ch1.tcycle(apu_reset_n);
        self.channels.ch2.tcycle(apu_reset_n);
        self.channels.ch3.tcycle(t_index, apu_reset_n);
        self.channels.ch4.tcycle(apu_reset_n);

        if !self.enabled {
            // Keep tracking the DIV-APU bit so we have the right edge
            // history when the APU is re-enabled.
            self.prev_div_apu_bit = div_counter & DIV_APU_BIT != 0;
            return;
        }

        let (l, r) = self.mix();
        self.sample_accum_left += l;
        self.sample_accum_right += r;
        self.sample_accum_count += 1;

        // Frame sequencer fires on falling edges of the DIV-APU bit.
        let div_apu_bit = div_counter & DIV_APU_BIT != 0;
        if self.prev_div_apu_bit && !div_apu_bit {
            self.tick_frame_sequencer();
        }
        self.prev_div_apu_bit = div_apu_bit;

        // Push the box-filtered average when the host sample window closes.
        self.sample_counter += 1.0;
        if self.sample_counter >= T_CYCLES_PER_SAMPLE {
            self.sample_counter -= T_CYCLES_PER_SAMPLE;
            let count = self.sample_accum_count as f32;
            self.sample_buffer.push((
                self.sample_accum_left / count,
                self.sample_accum_right / count,
            ));
            self.sample_accum_left = 0.0;
            self.sample_accum_right = 0.0;
            self.sample_accum_count = 0;
        }
    }

    /// Half-T-cycle audio work on master-clock fall (= apu_4mhz ↑ at
    /// mid-T-cycle). Drives CH3's BUSA and AZUS DFFs.
    pub fn fall_sync(&mut self) {
        if !self.enabled {
            return;
        }
        self.channels.ch3.fall_sync();
    }

    fn tick_frame_sequencer(&mut self) {
        // horu_512hz↑ (Family A) runs first so CH1/CH2 envelope-fire
        // latches (KOZY/JOPA) sample any kyvo armed by the previous
        // kene↓ before this step re-arms it — an NRx2 pace=0 write in
        // the intervening M-cycles clears kyvo and suppresses the fire.
        self.channels.ch1.sample_envelope_jopa();
        self.channels.ch2.sample_envelope_jopa();

        // bure↑ advances the (caru, bylu, JYNA) ripple; the strobes are
        // its bit-fall edges.
        self.frame_sequencer_step = (self.frame_sequencer_step + 1) % 8;
        let c = self.frame_sequencer_step;

        // caru↓ (bufy_256hz↓ → deme↑): C entered an even value.
        if c % 2 == 0 {
            self.channels.tick_length_all();
        }
        // bylu↓ (cate_128hz↓): arm coze; BEXA samples at next ajer↑
        // inside pulse_sweep::tcycle.
        if c == 0 || c == 4 {
            self.channels.ch1.tick_sweep_counter();
        }
        // JYNA↓ (kene↓, the 7→0 wrap): CH4 stays atomic; CH1/CH2 split
        // the counter advance from the JOPA sample above.
        if c == 0 {
            self.channels.ch1.tick_envelope_counter();
            self.channels.ch2.tick_envelope_counter();
            self.channels.ch4.tick_envelope();
        }
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
        let (left, right) = self.channels.mix();
        // Normalize over four channels and scale by master volume.
        (
            left / 4.0 * self.volume_left.percentage(),
            right / 4.0 * self.volume_right.percentage(),
        )
    }

    pub fn drain_samples(&mut self) -> Vec<(f32, f32)> {
        std::mem::take(&mut self.sample_buffer)
    }

    /// Construct an Audio instance from a gbtrace snapshot.
    #[cfg(feature = "gbtrace")]
    pub fn from_snapshot(snap: &gbtrace::snapshot::ApuSnapshot, wave_ram: [u8; 16]) -> Self {
        use channels::noise::FrequencyAndRandomness;
        use channels::registers::{
            PeriodDivider, Prescaler, Signed11, VolumeAndEnvelope, WaveformAndInitialLength,
        };
        use channels::wave::Volume as WaveVolume;
        use channels::{
            Enabled,
            noise::NoiseChannel,
            pulse::PulseChannel,
            pulse_sweep::{PulseSweepChannel, Sweep},
            wave::WaveChannel,
        };

        let channels = Channels {
            ch1: PulseSweepChannel {
                enabled: Enabled {
                    enabled: true,
                    output_left: true,
                    output_right: true,
                },
                sweep: Sweep(snap.ch1_sweep),
                waveform_and_initial_length: WaveformAndInitialLength(snap.ch1_duty_len),
                volume_and_envelope: VolumeAndEnvelope(snap.ch1_vol_env),
                length_enabled: snap.ch1_length_enabled,
                period: Signed11(snap.ch1_period),
                prescaler: Prescaler::default(),
                divider: PeriodDivider::default(),
                wave_duty_position: 0,
                pwm_latch: false,
                pending_trigger_sync: 0,
                divider_load_settle: false,
                current_volume: 0,
                envelope_timer: snap.ch1_envelope_timer,
                length_counter: 0,
                shadow_frequency: snap.ch1_period,
                sweep_timer: snap.ch1_sweep_timer,
                sweep_enabled: snap.ch1_sweep_enabled,
                sweep_negate_used: snap.ch1_sweep_negate_used,
                kyvo: false,
                coze: false,
            },
            ch2: PulseChannel {
                enabled: Enabled {
                    enabled: true,
                    output_left: true,
                    output_right: true,
                },
                waveform_and_initial_length: WaveformAndInitialLength(snap.ch2_duty_len),
                volume_and_envelope: VolumeAndEnvelope(snap.ch2_vol_env),
                length_enabled: snap.ch2_length_enabled,
                period: Signed11(snap.ch2_period),
                prescaler: Prescaler::default(),
                divider: PeriodDivider::default(),
                wave_duty_position: 0,
                pwm_latch: false,
                pending_trigger_sync: 0,
                divider_load_settle: false,
                current_volume: 0,
                envelope_timer: snap.ch2_envelope_timer,
                length_counter: 0,
                kyvo: false,
            },
            ch3: WaveChannel {
                enabled: Enabled {
                    enabled: true,
                    output_left: true,
                    output_right: true,
                },
                dac_enabled: snap.ch3_dac & 0x80 != 0,
                volume: WaveVolume(snap.ch3_vol),
                length_enabled: snap.ch3_length_enabled,
                period: Signed11(snap.ch3_period),
                ram: wave_ram,
                length_counter: 0,
                ch3_2mhz: false,
                frequency_timer: 0,
                wave_position: 0,
                ch3_fdis: true,
                ch3_frst: false,
                pending_overflow: false,
                trigger_sync: channels::wave::TriggerSync::default(),
                wave_data_latch: channels::wave::WaveDataLatch::default(),
            },
            ch4: NoiseChannel {
                enabled: Enabled {
                    enabled: true,
                    output_left: true,
                    output_right: true,
                },
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
            sample_accum_left: 0.0,
            sample_accum_right: 0.0,
            sample_accum_count: 0,
            sample_buffer: Vec::new(),
        }
    }
}
