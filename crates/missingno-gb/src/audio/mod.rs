use std::marker::PhantomData;

use channels::registers::Prescaler;
use channels::wave::WaveRamCoupling;
use channels::{Channels, noise, pulse, pulse_sweep, wave};
use volume::Volume;

pub mod channels;
pub mod registers;
pub mod volume;

/// Static, per-console APU properties. Each is a compile-time fact the DMG and
/// CGB silicon fix differently, so the console-specific runtime setters and
/// flag fields collapse into consts the monomorphization folds away.
pub trait ApuSpec {
    /// Console has the KEY1 ÷2 cell. When false the frame-sequencer double-rate
    /// tap, the CH1/CH2 prescaler free-run, and the CH4 cold-load DS term all
    /// dead-code (their runtime `double_speed` argument folds to false).
    const DOUBLE_SPEED: bool = false;
    /// CGB widens the CH1 sweep-counter load-hold by one ch1_1mhz↑ (`ch1_ld_sum`
    /// spans a second cycle); DMG keeps the single-cycle divider settle.
    const WIDE_SWEEP_LOAD_HOLD: bool = false;
    /// CGB grid-anchors the CH4 mid-run divisor-code reload cadence (the
    /// divisor prescaler reaches terminal K·new_code kanu steps past the last).
    const NOISE_GRID_ANCHOR: bool = false;
    /// How the CPU couples to CH3's wave SRAM while the channel is active.
    const WAVE_RAM_COUPLING: WaveRamCoupling = WaveRamCoupling::FetchStrobe;
}

/// DMG APU spec — every property at its default.
#[derive(Clone, Copy, Default)]
pub struct DmgApu;
impl ApuSpec for DmgApu {}

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
pub struct Audio<A: ApuSpec> {
    pub(crate) enabled: bool,
    /// The 1 MHz channel clock. Each pulse channel has its own CALO/AJER
    /// divider pair on the die, but they share the master clock and the
    /// power reset, so one counter serves CH1 and CH2.
    pub(crate) channel_clock: Prescaler,
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
    pending_left: i32,
    pending_right: i32,
    pending_count: u32,
    // The mix only changes when a channel flags output_dirty, so the
    // per-tcycle accumulation is run-length compressed: `last_mix`
    // held for `mix_run` T-cycles, flushed into pending_* on change.
    last_mix: (i32, i32),
    mix_run: u32,
    sample_accum_left: f32,
    sample_accum_right: f32,
    sample_accum_count: u32,
    sample_buffer: Vec<(f32, f32)>,
    _spec: PhantomData<A>,
}

impl<A: ApuSpec> Default for Audio<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: ApuSpec> Audio<A> {
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
            channel_clock: Prescaler::default(),
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
            last_mix: (0, 0),
            mix_run: 0,
            sample_accum_left: 0.0,
            sample_accum_right: 0.0,
            sample_accum_count: 0,
            sample_buffer: Vec::new(),
            _spec: PhantomData,
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
            channel_clock: Prescaler::default(),
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
            last_mix: (0, 0),
            mix_run: 0,
            sample_accum_left: 0.0,
            sample_accum_right: 0.0,
            sample_accum_count: 0,
            sample_buffer: Vec::new(),
            _spec: PhantomData,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The shared 1 MHz channel-clock phase (CALO/AJER counter).
    pub fn channel_clock_counter(&self) -> u8 {
        self.channel_clock.counter
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
        self.frame_sequencer_step.is_multiple_of(2)
    }

    pub fn prev_div_apu_bit(&self) -> bool {
        self.prev_div_apu_bit
    }

    /// One T-cycle of APU work, called at every master-clock rise.
    /// `apu_reset_n` is NR52 bit 7 — the channels' prescaler DFFs
    /// honour it as an async-reset, so we still call each tcycle
    /// unconditionally to keep the reset edge observable.
    pub fn tcycle(&mut self, div_counter: u16, t_index: u8, double_speed: bool) {
        // KEY1 is a runtime bit, but only a `DOUBLE_SPEED` console can raise it;
        // the DMG monomorphization folds this to false and dead-codes every
        // double-speed branch below (and inside the channels).
        let double_speed = A::DOUBLE_SPEED && double_speed;
        let div_apu_bit = if double_speed {
            DIV_APU_BIT_DOUBLE
        } else {
            DIV_APU_BIT
        };
        let apu_reset_n = self.enabled;
        // Detect the ripple edge before the channels tick: the strobes are
        // upstream on silicon, and the sweep cate↓ specifically must settle
        // before ch1's coincident divider wrap. Only the cate is delivered
        // early; the JOPA/step/length/envelope effects keep their late order.
        let mut fs_fire = false;
        if self.enabled {
            if self.fs_edge_pending {
                self.fs_edge_pending = false;
                fs_fire = true;
            }
            if self.fs_edge_predelay {
                self.fs_edge_predelay = false;
                self.fs_edge_pending = true;
            }
            let div_apu_high = div_counter & div_apu_bit != 0;
            if self.prev_div_apu_bit && !div_apu_high {
                if self.div_apu_switch_lag && double_speed {
                    self.fs_edge_predelay = true;
                } else if t_index >= 1 {
                    fs_fire = true;
                } else {
                    self.fs_edge_pending = true;
                }
            }
            self.prev_div_apu_bit = div_apu_high;
        }
        let c_next = (self.frame_sequencer_step + 1) % 8;
        let sweep_cate_due = fs_fire && (c_next == 0 || c_next == 4);
        let channel_clock_rose = self
            .channel_clock
            .tcycle(apu_reset_n, t_index, double_speed);
        let channel_clock_phase_one = self.channel_clock.counter == 1;
        self.channels.ch1.tcycle(
            apu_reset_n,
            channel_clock_rose,
            channel_clock_phase_one,
            A::WIDE_SWEEP_LOAD_HOLD,
            sweep_cate_due,
        );
        self.channels.ch2.tcycle(channel_clock_rose);
        self.channels.ch3.tcycle(apu_reset_n, A::WAVE_RAM_COUPLING);
        self.channels.ch4.tcycle(apu_reset_n, t_index, double_speed);

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

        if self.channels.take_output_dirty() {
            self.flush_mix_run();
            self.last_mix = self.channels.mix_dac();
        }
        self.mix_run += 1;
        debug_assert_eq!(self.last_mix, self.channels.mix_dac());

        // The ripple's remaining strobes land here, after the channels'
        // prescaler consume (a kene↓ inside an open load window is held).
        if fs_fire {
            self.tick_frame_sequencer();
        }

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

    /// Flush the run-length-compressed mix into the pending digital sums.
    fn flush_mix_run(&mut self) {
        if self.mix_run == 0 {
            return;
        }
        self.pending_left += self.last_mix.0 * self.mix_run as i32;
        self.pending_right += self.last_mix.1 * self.mix_run as i32;
        self.pending_count += self.mix_run;
        self.mix_run = 0;
    }

    /// Fold the pending DAC sums into the f32 accumulators at the current
    /// NR50 volume. Each powered DAC swings ±15 half-LSB units, four
    /// channels per side, so full scale is ±60.
    pub(crate) fn fold_pending(&mut self) {
        self.flush_mix_run();
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
        if c.is_multiple_of(2) {
            self.channels.tick_length_all();
            // Envelope-enable bug (enable on an odd step): the next even DIV-APU
            // tick advances the envelope counter. An enable on an even step is
            // handled at the write (that step's tick already ran). On c==0 the
            // kene↓ tick below does it.
            if self.channels.ch1.take_envelope_enable_tick_pending() && c != 0 {
                self.channels.ch1.tick_envelope_counter();
            }
            if self.channels.ch2.take_envelope_enable_tick_pending() && c != 0 {
                self.channels.ch2.tick_envelope_counter();
            }
        }
        // bylu↓ (cate_128hz↓): arm coze; BEXA samples at next ajer↑ inside
        // pulse_sweep::tcycle — unless the early wrap-coincident path already
        // delivered this rise's cate.
        if (c == 0 || c == 4) && !self.channels.ch1.take_sweep_cate() {
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
        let double_speed = A::DOUBLE_SPEED && double_speed;
        let div_apu_bit = if double_speed {
            DIV_APU_BIT_DOUBLE
        } else {
            DIV_APU_BIT
        };
        // The DIV-write 1→0 edge on the tap bit clocks the ripple — BURE↑ trails the
        // reset edge by ≈1 T-cycle (the same gate behaviour as a free-running tap fall),
        // so arm the edge for the next tcycle rather than firing synchronously here.
        let fire = self.enabled && old_counter & div_apu_bit != 0;
        self.prev_div_apu_bit = false; // counter is now 0, both taps clear
        if fire && self.div_apu_switch_lag && double_speed {
            self.fs_edge_predelay = true;
            self.fs_edge_pending = false;
        } else {
            self.fs_edge_pending = fire;
            self.fs_edge_predelay = false;
        }
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
    pub fn from_snapshot(snap: &gbtrace::family::gb::snapshot::ApuSnapshot, wave_ram: [u8; 16]) -> Self {
        use channels::noise::FrequencyAndRandomness;
        use channels::registers::{
            PeriodDivider, Prescaler, Signed11, VolumeAndEnvelope, WaveformAndInitialLength,
        };
        use channels::wave::Volume as WaveVolume;
        use channels::{
            Enabled, TriggerReload,
            envelope::Envelope,
            length::LengthCounter,
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
                length: LengthCounter {
                    enabled: snap.ch1_length_enabled,
                    counter: 0,
                },
                period: Signed11(snap.ch1_period),
                divider: PeriodDivider::default(),
                wave_duty_position: 0,
                pwm_latch: false,
                pending_reload: TriggerReload::Idle,
                divider_load_settle: false,
                sweep_load_hold: 0,
                envelope: Envelope {
                    timer: snap.ch1_envelope_timer,
                    ..Envelope::default()
                },
                shadow_frequency: snap.ch1_period,
                sweep_timer: snap.ch1_sweep_timer,
                sweep_enabled: snap.ch1_sweep_enabled,
                sweep_negate_used: snap.ch1_sweep_negate_used,
                coze: false,
                sweep_cate_taken: false,
                sweep_calc_steps: 0,
                sweep_calc_restart: false,
                ch1_frst: false,
                output_dirty: true,
            },
            ch2: PulseChannel {
                enabled: Enabled {
                    enabled: true,
                    output_left: true,
                    output_right: true,
                },
                waveform_and_initial_length: WaveformAndInitialLength(snap.ch2_duty_len),
                volume_and_envelope: VolumeAndEnvelope(snap.ch2_vol_env),
                length: LengthCounter {
                    enabled: snap.ch2_length_enabled,
                    counter: 0,
                },
                period: Signed11(snap.ch2_period),
                divider: PeriodDivider::default(),
                wave_duty_position: 0,
                pwm_latch: false,
                ch2_frst: false,
                pending_reload: TriggerReload::Idle,
                divider_load_settle: false,
                envelope: Envelope {
                    timer: snap.ch2_envelope_timer,
                    ..Envelope::default()
                },
                output_dirty: true,
            },
            ch3: WaveChannel {
                enabled: Enabled {
                    enabled: true,
                    output_left: true,
                    output_right: true,
                },
                dac_enabled: snap.ch3_dac & 0x80 != 0,
                volume: WaveVolume(snap.ch3_vol),
                length: LengthCounter {
                    enabled: snap.ch3_length_enabled,
                    counter: 0,
                },
                period: Signed11(snap.ch3_period),
                ram: wave_ram,
                ch3_2mhz: false,
                frequency_timer: 0,
                wave_position: 0,
                ch3_fdis: true,
                ch3_frst: false,
                pending_overflow: false,
                trigger_sync: channels::wave::TriggerSync::default(),
                wave_data_latch: channels::wave::WaveDataLatch::default(),
                sample_byte: 0,
                output_dirty: true,
            },
            ch4: NoiseChannel {
                enabled: Enabled {
                    enabled: true,
                    output_left: true,
                    output_right: true,
                },
                volume_and_envelope: VolumeAndEnvelope(snap.ch4_vol_env),
                length: LengthCounter {
                    enabled: snap.ch4_length_enabled,
                    counter: 0,
                },
                frequency_and_randomness: FrequencyAndRandomness(snap.ch4_freq),
                divider: 0,
                divider_subcounter: 0,
                prescaler: 0,
                gary: false,
                sync_delay: 0,
                prev_tap: false,
                mhz_prescaler: Prescaler::default(),
                jeso: false,
                double_speed: false,
                skip_first_clock: false,
                lfsr: 0x7FFF,
                envelope: Envelope {
                    timer: snap.ch4_envelope_timer,
                    ..Envelope::default()
                },
                output_dirty: true,
            },
        };

        Self {
            enabled: snap.sound_on & 0x80 != 0,
            channel_clock: Prescaler::default(),
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
            last_mix: (0, 0),
            mix_run: 0,
            sample_accum_left: 0.0,
            sample_accum_right: 0.0,
            sample_accum_count: 0,
            sample_buffer: Vec::new(),
            _spec: PhantomData,
        }
    }
}
