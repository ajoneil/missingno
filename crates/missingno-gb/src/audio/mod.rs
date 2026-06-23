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
                                  // In double speed the M-cycle counter runs at 2× the dot clock, so the tap
                                  // shifts up one bit to hold the frame sequencer at 512 Hz (DIV bit 6 vs bit 5).
const DIV_APU_BIT_DOUBLE: u16 = 1 << 11;

#[derive(Clone)]
pub struct Audio {
    pub(crate) enabled: bool,
    pub(crate) channels: Channels,
    pub(crate) volume_left: Volume,
    pub(crate) volume_right: Volume,
    pub(crate) nr50: u8,

    pub(crate) prev_div_apu_bit: bool,
    pub(crate) frame_sequencer_step: u8,
    // DIV-APU bit-10 fell last tcycle; the (caru, bylu, JYNA) ripple strobes
    // land one tcycle later (kylo/kene_inst buffer delay) — kene↓ in T1, not T0.
    pub(crate) fs_edge_pending: bool,
    // The →double tap retune slips the DIV-APU edge one M-cycle when the →double
    // count is odd. `parity` tracks that low bit; `lag` is the active slip set at
    // resume; `predelay` carries an armed edge one extra tcycle so the strobe lands
    // a cycle later (real divider used for detection — no view-shift artifacts).
    pub(crate) div_apu_double_parity: bool,
    pub(crate) div_apu_switch_lag: bool,
    pub(crate) fs_edge_predelay: bool,
    sample_counter: f32,
    // Digital channel sums accumulate as integers; fold_pending() applies
    // the DAC scale and NR50 volume when either changes or a window closes.
    pending_left: u32,
    pending_right: u32,
    pending_count: u32,
    sample_accum_left: f32,
    sample_accum_right: f32,
    sample_accum_count: u32,
    sample_buffer: Vec<(f32, f32)>,
}

impl Audio {
    /// Override CH1's post-boot duty/divider phase. The boot chime leaves CH1
    /// free-running with the duty position un-reset across triggers; the CGB
    /// boot ROM's chime ends at a different phase than the DMG one, which the
    /// `Default` channel state encodes.
    pub fn set_ch1_post_boot_phase(&mut self, wave_duty_position: u8, divider: u16) {
        self.channels.ch1.wave_duty_position = wave_duty_position;
        self.channels.ch1.divider.counter = divider;
    }

    /// Post-boot state at PC=0x0100. `prev_div_apu_bit` derives from the
    /// M-cycle `reg_div16` (the ripple advance stays divider-locked). The
    /// (caru, bylu, JYNA) frame-sequencer ripple is apu_reset-reset, so its
    /// phase is the boot ROM's leftover — kene↓ fires at reg_div16≡0x1800,
    /// not at the divider phase (reg_div16>>11)&7.
    pub fn post_boot(internal_counter: u16) -> Self {
        Self {
            enabled: true,
            channels: Channels::default(),
            volume_left: Volume::max(),
            volume_right: Volume::max(),
            nr50: 0x77,

            prev_div_apu_bit: internal_counter & DIV_APU_BIT != 0,
            // Boot ROM leftover ripple phase: step 0 (kene↓) lands at
            // reg_div16≡0x1800, three advances past the divider's 0.
            frame_sequencer_step: 2,
            fs_edge_pending: false,
            div_apu_double_parity: false,
            div_apu_switch_lag: false,
            fs_edge_predelay: false,
            sample_counter: 0.0,
            pending_left: 0,
            pending_right: 0,
            pending_count: 0,
            sample_accum_left: 0.0,
            sample_accum_right: 0.0,
            sample_accum_count: 0,
            sample_buffer: Vec::new(),
        }
    }

    /// Post-boot state seeded with an explicit frame-sequencer step — each
    /// console's boot ROM hands off at a different DIV-APU phase. The plain
    /// `post_boot` is the DMG handoff (step 2).
    pub fn post_boot_with_fs_step(internal_counter: u16, frame_sequencer_step: u8) -> Self {
        let mut audio = Self::post_boot(internal_counter);
        audio.frame_sequencer_step = frame_sequencer_step;
        audio
    }

    /// Power-on state: audio disabled, all registers zeroed.
    pub fn new() -> Self {
        let mut channels = Channels::default();
        channels.reset_all();
        Self {
            enabled: false,
            channels,
            volume_left: Volume(0),
            volume_right: Volume(0),
            nr50: 0x00,
            prev_div_apu_bit: false, // internal_counter starts at 0, bit 12 = 0
            frame_sequencer_step: 0,
            fs_edge_pending: false,
            div_apu_double_parity: false,
            div_apu_switch_lag: false,
            fs_edge_predelay: false,
            sample_counter: 0.0,
            pending_left: 0,
            pending_right: 0,
            pending_count: 0,
            sample_accum_left: 0.0,
            sample_accum_right: 0.0,
            sample_accum_count: 0,
            sample_buffer: Vec::new(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// PCM12: CGB-only digital tap of the channel DACs — CH1 low nibble, CH2 high.
    pub fn pcm12(&self) -> u8 {
        self.channels.ch1.digital_sample() | (self.channels.ch2.digital_sample() << 4)
    }

    /// PCM34: CH3 low nibble, CH4 high.
    pub fn pcm34(&self) -> u8 {
        self.channels.ch3.digital_sample() | (self.channels.ch4.digital_sample() << 4)
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
    pub fn tcycle(
        &mut self,
        div_counter: u16,
        t_index: u8,
        double_speed: bool,
        wave_ram_coupling: wave::WaveRamCoupling,
    ) {
        let div_apu_bit = if double_speed {
            DIV_APU_BIT_DOUBLE
        } else {
            DIV_APU_BIT
        };
        let apu_reset_n = self.enabled;
        self.channels.ch1.tcycle(apu_reset_n, t_index, double_speed);
        self.channels.ch2.tcycle(apu_reset_n, t_index, double_speed);
        self.channels.ch3.tcycle(apu_reset_n, wave_ram_coupling);
        self.channels.ch4.tcycle(apu_reset_n);

        if !self.enabled {
            // Keep tracking the DIV-APU bit so we have the right edge
            // history when the APU is re-enabled. Power-off resets the
            // frame sequencer, so drop any armed ripple edge.
            self.prev_div_apu_bit = div_counter & div_apu_bit != 0;
            self.fs_edge_pending = false;
            self.fs_edge_predelay = false;
            // Power-off re-locks the frame sequencer, so the →double tap-retune
            // slip and its parity are cleared too.
            self.div_apu_switch_lag = false;
            self.div_apu_double_parity = false;
            return;
        }

        let (l, r) = self.channels.mix_digital();
        self.pending_left += l;
        self.pending_right += r;
        self.pending_count += 1;

        // Fire the ripple edge armed last tcycle (the strobes land one tcycle
        // after the bit-10 fall). It runs after the prescaler consume above set
        // divider_load_settle, so a kene↓ inside the open load window is held.
        if self.fs_edge_pending {
            self.fs_edge_pending = false;
            self.tick_frame_sequencer();
        }
        // A →double-slipped edge waits one extra tcycle: last tcycle's predelay
        // becomes this tcycle's pending, so the strobe lands a cycle later.
        if self.fs_edge_predelay {
            self.fs_edge_predelay = false;
            self.fs_edge_pending = true;
        }

        // DIV-APU bit-10 fall arms the ripple advance for next tcycle — via the
        // extra predelay stage while the →double tap-retune slip holds.
        let div_apu_high = div_counter & div_apu_bit != 0;
        if self.prev_div_apu_bit && !div_apu_high {
            if self.div_apu_switch_lag && double_speed {
                self.fs_edge_predelay = true;
            } else {
                self.fs_edge_pending = true;
            }
        }
        self.prev_div_apu_bit = div_apu_high;

        // Push the box-filtered average when the host sample window closes.
        self.sample_counter += 1.0;
        if self.sample_counter >= T_CYCLES_PER_SAMPLE {
            self.sample_counter -= T_CYCLES_PER_SAMPLE;
            self.fold_pending();
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

    /// CH3 `foba` arm capture, clocked by `apu_phi↑` (the CPU M-cycle boundary).
    /// Gated by APU power, like the per-dot tick's `apu_reset_n`.
    pub fn mcycle_boundary(&mut self) {
        if self.enabled {
            self.channels.ch3.arm_trigger();
        }
    }

    /// Fold the pending digital sums into the f32 accumulators at the
    /// current NR50 volume. Channels span 0–15 across four channels per
    /// side, so full scale is 60.
    pub(crate) fn fold_pending(&mut self) {
        if self.pending_count == 0 {
            return;
        }
        const FULL_SCALE: f32 = 1.0 / 60.0;
        self.sample_accum_left +=
            self.pending_left as f32 * FULL_SCALE * self.volume_left.percentage();
        self.sample_accum_right +=
            self.pending_right as f32 * FULL_SCALE * self.volume_right.percentage();
        self.sample_accum_count += self.pending_count;
        self.pending_left = 0;
        self.pending_right = 0;
        self.pending_count = 0;
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
        self.channels.ch4.sample_envelope_jopa();

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
            self.channels.ch4.tick_envelope_counter();
        }
    }

    /// Called when DIV resets the internal counter to 0 (FF04 write or KEY1 speed
    /// switch). If the DIV-APU tap bit for the divider's speed was set, the 1→0 edge
    /// ticks the frame sequencer. `double_speed` is the speed in effect for
    /// `old_counter` — the PRE-switch speed on the speed-switch path.
    pub fn on_div_write(&mut self, old_counter: u16, double_speed: bool) {
        let div_apu_bit = if double_speed {
            DIV_APU_BIT_DOUBLE
        } else {
            DIV_APU_BIT
        };
        if self.enabled && old_counter & div_apu_bit != 0 {
            self.tick_frame_sequencer();
        }
        self.prev_div_apu_bit = false; // counter is now 0, both taps clear
        self.fs_edge_pending = false; // divider reset supersedes any armed tap edge
        self.fs_edge_predelay = false; // and any slipped edge still in its extra cycle
    }

    /// KEY1 entry: a →double swap toggles the tap-retune parity (the slip is
    /// present when the →double count is odd); the active slip is dropped for the
    /// blackout and reinstated from the parity at resume.
    pub fn on_speed_switch(&mut self, to_double: bool) {
        if to_double {
            self.div_apu_double_parity = !self.div_apu_double_parity;
        }
        self.div_apu_switch_lag = false;
    }

    /// Blackout resume: apply the tap-retune slip for the current →double parity.
    pub fn on_speed_resume(&mut self) {
        self.div_apu_switch_lag = self.div_apu_double_parity;
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
            noise::NoiseChannel,
            pulse::PulseChannel,
            pulse_sweep::{PulseSweepChannel, Sweep},
            wave::WaveChannel,
            Enabled,
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
                envelope_stopped: false,
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
                envelope_stopped: false,
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
                sample_byte: 0,
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
                skip_first_clock: false,
                lfsr: 0x7FFF,
                current_volume: 0,
                envelope_timer: snap.ch4_envelope_timer,
                envelope_stopped: false,
                kyvo: false,
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
            fs_edge_pending: false,
            div_apu_double_parity: false,
            div_apu_switch_lag: false,
            fs_edge_predelay: false,
            sample_counter: 0.0,
            pending_left: 0,
            pending_right: 0,
            pending_count: 0,
            sample_accum_left: 0.0,
            sample_accum_right: 0.0,
            sample_accum_count: 0,
            sample_buffer: Vec::new(),
        }
    }
}
