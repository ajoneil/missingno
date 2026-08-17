use super::{
    Enabled, TriggerReload,
    envelope::Envelope,
    length::LengthCounter,
    registers::{
        PeriodDivider, PeriodHighAndControl, Signed11, VolumeAndEnvelope, WaveformAndInitialLength,
    },
    sweep::{NoSweep, PeriodSweep, Sweep, SweepUnit},
};

const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 1, 0], // 12.5%
    [0, 0, 0, 0, 0, 0, 1, 1], // 25%
    [0, 0, 0, 0, 1, 1, 1, 1], // 50%
    [1, 1, 1, 1, 1, 1, 0, 0], // 75%
];

/// NR11-NR14 on CH1, NR21-NR24 on CH2. CH1's sweep register has no CH2
/// counterpart, so it is written through the sweep unit instead.
#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    WaveformAndInitialLength,
    VolumeAndEnvelope,
    PeriodLow,
    PeriodHighAndControl,
}

/// A pulse channel: the 11-bit period divider driving a duty pipeline, the
/// length counter and the volume envelope. CH1 is this silicon carrying a
/// [`Sweep`]; CH2 is the same carrying [`NoSweep`].
#[derive(Clone)]
pub struct PulseChannel<S: SweepUnit> {
    pub enabled: Enabled,
    pub sweep: S,
    pub waveform_and_initial_length: WaveformAndInitialLength,
    pub volume_and_envelope: VolumeAndEnvelope,
    pub length: LengthCounter<64>,
    pub period: Signed11,

    pub divider: PeriodDivider,
    pub wave_duty_position: u8,
    /// `duwo`/`dome` PWM latch — captures the duty-pattern bit on each
    /// natural-overflow `chN_frst↑`; holds the emitted output between
    /// overflows.
    pub pwm_latch: bool,
    /// `ch1_frst`/`ch2_frst` overflow pulse — high for one `chN_1mhz` cycle
    /// after an overflow. `duwo`/`dome` captures the pre-advance duty on its
    /// rise (the overflow edge); the duty counter (`dajo`/`cule`) clocks on its
    /// fall, one cycle later. So capture precedes advance.
    pub overflow_pulse: bool,
    /// `chN_restart` sync stage; pending between the NRx4 trigger write
    /// and the next chN_1mhz↑ that applies the reload.
    pub pending_reload: TriggerReload,
    /// Set on the reload edge; the first count is suppressed so the
    /// divider DFFs settle out of load mode before counting resumes.
    pub divider_load_settle: bool,
    pub envelope: Envelope,
    /// An input to `digital_sample()` / the mix may have changed.
    pub output_dirty: bool,
}

impl Default for PulseChannel<Sweep> {
    fn default() -> Self {
        // Post-boot state at PC=0x0100 (boot ROM's Nintendo chime ran
        // CH1 to a known mid-period state with the envelope decayed).
        Self {
            enabled: Enabled {
                enabled: true,
                output_left: true,
                output_right: true,
            },
            sweep: Sweep {
                register: PeriodSweep(0x80),
                ..Sweep::default()
            },
            waveform_and_initial_length: WaveformAndInitialLength(0xbf),
            volume_and_envelope: VolumeAndEnvelope(0xf3),
            period: Signed11(0x7C1),
            divider: PeriodDivider { counter: 0x7F9 },
            wave_duty_position: 2,
            // chime decay ran to saturation; JEME latched
            envelope: Envelope {
                stopped: true,
                ..Envelope::default()
            },
            ..Self::at_reset(0)
        }
    }
}

impl Default for PulseChannel<NoSweep> {
    fn default() -> Self {
        // Post-boot state at PC=0x0100. Boot ROM doesn't drive CH2: DAC off,
        // channel disabled, internal counters at reset (NR23/NR24 never
        // written, so acc_d = 0).
        Self {
            enabled: Enabled {
                enabled: false, // ch2_fdis = 1 (channel disabled)
                output_left: true,
                output_right: true,
            },
            waveform_and_initial_length: WaveformAndInitialLength(0x3f),
            ..Self::at_reset(0)
        }
    }
}

impl<S: SweepUnit> PulseChannel<S> {
    /// apu_reset: every register cleared and every internal counter at rest.
    /// DMG preserves the length timer across power-off, so it is carried in.
    fn at_reset(length_counter: u16) -> Self {
        Self {
            enabled: Enabled::disabled(),
            sweep: S::default(),
            waveform_and_initial_length: WaveformAndInitialLength(0),
            volume_and_envelope: VolumeAndEnvelope(0),
            length: LengthCounter {
                enabled: false,
                counter: length_counter,
            },
            period: (0).into(),

            divider: PeriodDivider::default(),
            wave_duty_position: 0,
            pwm_latch: false,
            overflow_pulse: false,
            pending_reload: TriggerReload::Idle,
            divider_load_settle: false,
            envelope: Envelope::default(),
            output_dirty: true,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::at_reset(self.length.counter);
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::WaveformAndInitialLength => self.waveform_and_initial_length.0 | 0x3F,
            Register::VolumeAndEnvelope => self.volume_and_envelope.0,
            Register::PeriodLow => 0xff,
            Register::PeriodHighAndControl => PeriodHighAndControl::read(self.length.enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8, length_clock_low: bool) {
        self.output_dirty = true;
        match register {
            Register::WaveformAndInitialLength => {
                self.waveform_and_initial_length = WaveformAndInitialLength(value);
                self.length
                    .load(self.waveform_and_initial_length.initial_length() as u16);
            }
            Register::VolumeAndEnvelope => {
                // Write-strobe transient: the pace bits read 1 while the
                // cells settle, so JUPU dips iff the old pace was 0 and
                // HOFO completes one pulse — one +1 volume clock, free
                // 4-bit wrap (JEME never latches under pace 0).
                let old_pace = self.volume_and_envelope.sweep_pace();
                self.envelope.zombie_bump(old_pace);
                self.volume_and_envelope = VolumeAndEnvelope(value);
                let new_pace = self.volume_and_envelope.sweep_pace();
                // Turning the envelope on (pace 0→non-zero) on a running
                // channel makes the next even DIV-APU tick advance the envelope
                // counter. If this write lands on an even step its tick already
                // ran, so apply it now; otherwise defer to the next even step.
                if old_pace == 0 && new_pace != 0 && self.enabled.enabled {
                    if length_clock_low {
                        self.tick_envelope_counter();
                    } else {
                        self.envelope.enable_tick_pending = true;
                    }
                }
                // pace=0 raises jupu → hafe=0 → KOZY/JOPA async-reset; any
                // armed kyvo is dropped before the next horu_512hz↑.
                if new_pace == 0 {
                    self.envelope.saturation_armed = false;
                }
                // Disabling the DAC immediately disables the channel
                if value & 0xf8 == 0 {
                    self.enabled.enabled = false;
                }
            }
            Register::PeriodLow => self.period.set_low8(value),
            Register::PeriodHighAndControl => {
                let ctrl = PeriodHighAndControl(value);
                self.period.set_high3(ctrl.period_high());

                // capy/deme = NOR(cero/cyre, bufy_256hz, NRx4 d6): length-enable
                // 0→1 rises it (one extra length count) iff caru is low.
                if self
                    .length
                    .enable_glitch(length_clock_low, ctrl.enable_length(), ctrl.trigger())
                {
                    self.enabled.enabled = false;
                }

                if ctrl.trigger() {
                    self.trigger();
                    self.length.trigger_enable_fixup(length_clock_low);
                }
            }
        }
    }

    pub fn trigger(&mut self) {
        // chN_fdis (set by DAC-off / apu_reset, cleared by a trigger) gates the
        // divider toggle clock. Only the channel-enabling trigger — the one that
        // clears fdis 1→0 — freezes a load tick (the +1 first overflow); a
        // re-trigger of a running channel reloads with no +1 (the reload arm
        // distinguishes the enabling case).
        let was_running = self.enabled.enabled;
        self.enabled.enabled = true;
        self.length.trigger_reload();
        // Arm the chN_restart sync: the reload applies at the next
        // chN_1mhz↑, not on this write edge. A coincident natural
        // overflow on that wrap is suppressed (on CH1, dyru async-resets
        // comy before cala can clock).
        self.pending_reload = if was_running {
            TriggerReload::Retrigger
        } else {
            TriggerReload::Enabling
        };
        // chN_restart pulls hafe low → KOZY/JOPA reset → any prior kyvo
        // arm from the previous trigger window is dropped.
        self.envelope.trigger(
            self.volume_and_envelope.initial_volume(),
            self.volume_and_envelope.sweep_pace(),
        );
        self.sweep.on_trigger(self.period.0);

        // DAC check: if upper 5 bits of volume register are 0, channel is disabled
        if !self.dac_enabled() {
            self.enabled.enabled = false;
        }
    }

    /// One chN_1mhz↑ of the period divider and its duty pipeline.
    /// `wide_sweep_hold` reaches only a channel that carries a sweep unit.
    pub(super) fn tick_divider(&mut self, channel_clock_rose: bool, wide_sweep_hold: bool) {
        if !channel_clock_rose || !self.enabled.enabled {
            return;
        }
        self.sweep.on_channel_clock();
        // chN_frst↓ (one chN_1mhz↑ after an overflow): the duty counter
        // (dajo/cule) clocks on the fall, so the advance trails duwo/dome's
        // capture by one cycle.
        if self.overflow_pulse {
            self.wave_duty_position = (self.wave_duty_position + 1) % 8;
            self.overflow_pulse = false;
        }
        // Prescaler wrapped (chN_1mhz↑). Trigger reload and natural
        // overflow are mutually exclusive on the same edge — trigger
        // wins via dyru's async-reset of comy.
        if self.pending_reload != TriggerReload::Idle {
            // Enabling trigger freezes the load tick → +1 first overflow;
            // re-trigger reloads with no +1.
            self.divider_load_settle = self.pending_reload == TriggerReload::Enabling;
            self.sweep.on_trigger_reload(wide_sweep_hold);
            self.divider.counter = (self.period.0) & 0x7FF;
            self.pending_reload = TriggerReload::Idle;
        } else if self.divider_load_settle {
            self.divider_load_settle = false;
        } else if self.divider.counter >= 0x7FF {
            // chN_frst↑ (the overflow): duwo/dome captures the pre-advance duty
            // step and the divider reloads; the counter advances next cycle.
            let duty = self.waveform_and_initial_length.waveform() as usize;
            let latch = DUTY_TABLE[duty][self.wave_duty_position as usize] != 0;
            if latch != self.pwm_latch {
                self.pwm_latch = latch;
                self.output_dirty = true;
            }
            self.overflow_pulse = true;
            self.divider.counter = (self.period.0) & 0x7FF;
        } else {
            self.divider.counter += 1;
        }
    }

    pub fn tick_length(&mut self) {
        if self.length.tick() {
            self.enabled.enabled = false;
            self.output_dirty = true;
        }
    }

    /// Consume the envelope-enable-bug arm set by the last enabling NRx2
    /// write; the caller advances the envelope counter on the even tick.
    pub fn take_envelope_enable_tick_pending(&mut self) -> bool {
        self.envelope.take_enable_tick_pending()
    }

    /// kene↓ edge (fs step 7→0). Advances the envelope counter and
    /// arms `kyvo` on saturation; the volume update is deferred to the
    /// next horu_512hz↑ sample so a same-step NRx2 pace=0 write can
    /// clear `kyvo` and suppress the fire.
    pub fn tick_envelope_counter(&mut self) {
        self.envelope.tick_counter(
            self.volume_and_envelope.sweep_pace(),
            self.divider_load_settle,
        );
    }

    /// JOPA sample on the horu_512hz↑ edge (every fs step transition). Drains
    /// `kyvo` into the volume counter when `hafe` is asserted; otherwise
    /// consumes `kyvo` without firing (= dropped sample).
    pub fn sample_envelope_fire(&mut self) {
        if self.envelope.sample_fire(
            self.volume_and_envelope.sweep_pace(),
            self.enabled.enabled,
            self.volume_and_envelope.direction(),
        ) {
            self.output_dirty = true;
        }
    }

    // DAC power = NRx2 upper five bits (HOCA on CH1, FUTE on CH2).
    pub fn dac_enabled(&self) -> bool {
        self.volume_and_envelope.0 & 0xf8 != 0
    }

    /// The bit `duwo`/`dome` would capture at duty step `position`.
    pub fn duty_bit(&self, position: u8) -> bool {
        let duty = self.waveform_and_initial_length.waveform() as usize;
        DUTY_TABLE[duty][(position % 8) as usize] != 0
    }

    pub fn digital_sample(&self) -> u8 {
        if !self.enabled.enabled {
            return 0;
        }
        // The DAC sees the latched duty bit from the previous
        // overflow, not the combinational chN_pwm output.
        if self.pwm_latch {
            self.envelope.volume
        } else {
            0
        }
    }
}

impl PulseChannel<NoSweep> {
    /// One master-clock rise. `channel_clock_rose` is the shared CALO↑
    /// (ch2_1mhz↑) strobe, low while apu_reset holds the clock.
    pub fn tcycle(&mut self, channel_clock_rose: bool) {
        self.tick_divider(channel_clock_rose, false);
    }
}
