use super::{
    Enabled, TriggerReload,
    envelope::Envelope,
    length::LengthCounter,
    registers::{
        PeriodDivider, PeriodHighAndControl, Signed11, VolumeAndEnvelope, WaveformAndInitialLength,
    },
};

const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 1, 0], // 12.5%
    [0, 0, 0, 0, 0, 0, 1, 1], // 25%
    [0, 0, 0, 0, 1, 1, 1, 1], // 50%
    [1, 1, 1, 1, 1, 1, 0, 0], // 75%
];

#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    WaveformAndInitialLength,
    VolumeAndEnvelope,
    PeriodLow,
    PeriodHighAndControl,
}

#[derive(Clone)]
#[cfg_attr(debug_assertions, derive(Debug, PartialEq))]
pub struct PulseChannel {
    pub enabled: Enabled,
    pub waveform_and_initial_length: WaveformAndInitialLength,
    pub volume_and_envelope: VolumeAndEnvelope,
    pub length: LengthCounter<64>,
    pub period: Signed11,

    pub divider: PeriodDivider,
    pub wave_duty_position: u8,
    /// `dome` PWM latch (CH2 mirror of CH1's `duwo`).
    pub pwm_latch: bool,
    /// `ch2_frst` overflow pulse — high for one `ch2_1mhz` cycle after an
    /// overflow. `dome` captures the pre-advance duty on its rise; the duty
    /// counter (`cule`) clocks on its fall, one cycle later (CH2 mirror of
    /// CH1's `ch1_frst`).
    pub ch2_frst: bool,
    /// `ch2_restart` sync stage; pending between NR24 trigger write
    /// and the next ch2_1mhz↑ that applies the reload.
    pub pending_reload: TriggerReload,
    /// Set on the reload edge; the first count is suppressed so the
    /// divider DFFs settle out of load mode (CH1/CH2 mirror).
    pub divider_load_settle: bool,
    pub envelope: Envelope,
    /// An input to `digital_sample()` / the mix may have changed.
    pub output_dirty: bool,
}

impl Default for PulseChannel {
    fn default() -> Self {
        // Post-boot state at PC=0x0100. Boot ROM doesn't drive CH2:
        // DAC off, channel disabled, internal counters at reset.
        Self {
            enabled: Enabled {
                enabled: false, // ch2_fdis = 1 (channel disabled)
                output_left: true,
                output_right: true,
            },
            waveform_and_initial_length: WaveformAndInitialLength(0x3f),
            volume_and_envelope: VolumeAndEnvelope(0),
            length: LengthCounter::default(),
            period: Signed11(0), // CH2 NR23/NR24 never written by boot ROM; acc_d = 0

            divider: PeriodDivider::default(),
            wave_duty_position: 0,
            pwm_latch: false,
            ch2_frst: false,
            pending_reload: TriggerReload::Idle,
            divider_load_settle: false,
            envelope: Envelope::default(),
            output_dirty: true,
        }
    }
}

impl PulseChannel {
    pub fn reset(&mut self) {
        let length_counter = self.length.counter; // DMG: length timers preserved on power-off
        *self = Self {
            enabled: Enabled::disabled(),
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
            ch2_frst: false,
            pending_reload: TriggerReload::Idle,
            divider_load_settle: false,
            envelope: Envelope::default(),
            output_dirty: true,
        };
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::WaveformAndInitialLength => self.waveform_and_initial_length.0 | 0x3F,
            Register::VolumeAndEnvelope => self.volume_and_envelope.0,
            Register::PeriodLow => 0xff,
            Register::PeriodHighAndControl => PeriodHighAndControl::read(self.length.enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8, caru_low: bool) {
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
                    if caru_low {
                        self.tick_envelope_counter();
                    } else {
                        self.envelope.enable_tick_pending = true;
                    }
                }
                // pace=0 raises jupu → hafe=0 → JOPA async-reset; any
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

                // deme = NOR(cyre, bufy_256hz, ff19_d6_n): length-enable
                // 0→1 rises deme (one extra length count) iff caru is low.
                if self
                    .length
                    .enable_glitch(caru_low, ctrl.enable_length(), ctrl.trigger())
                {
                    self.enabled.enabled = false;
                }

                if ctrl.trigger() {
                    self.trigger();
                    self.length.trigger_enable_fixup(caru_low);
                }
            }
        }
    }

    pub fn trigger(&mut self) {
        // Only the channel-enabling trigger (ch2_fdis 1→0) freezes the load tick
        // (the +1 first overflow); a re-trigger of a running channel reloads with
        // no +1.
        let was_running = self.enabled.enabled;
        self.enabled.enabled = true;
        self.length.trigger_reload();
        // Arm the ch2_restart sync: the reload applies at the next
        // ch2_1mhz↑, not on this write edge.
        self.pending_reload = if was_running {
            TriggerReload::Retrigger
        } else {
            TriggerReload::Enabling
        };
        // ch2_restart pulls hafe low → JOPA reset → any prior kyvo
        // arm from the previous trigger window is dropped.
        self.envelope.trigger(
            self.volume_and_envelope.initial_volume(),
            self.volume_and_envelope.sweep_pace(),
        );

        // DAC check: if upper 5 bits of volume register are 0, channel is disabled
        if !self.dac_enabled() {
            self.enabled.enabled = false;
        }
    }

    /// One master-clock rise. `channel_clock_rose` is the shared CALO↑
    /// (ch2_1mhz↑) strobe, low while apu_reset holds the clock.
    pub fn tcycle(&mut self, channel_clock_rose: bool) {
        if !channel_clock_rose || !self.enabled.enabled {
            return;
        }
        // ch2_frst↓ (one ch2_1mhz↑ after an overflow): the duty counter (cule)
        // clocks on the fall, so the advance trails dome's capture by one cycle.
        if self.ch2_frst {
            self.wave_duty_position = (self.wave_duty_position + 1) % 8;
            self.ch2_frst = false;
        }
        if self.pending_reload != TriggerReload::Idle {
            // Enabling trigger freezes the load tick → +1 first overflow;
            // re-trigger reloads with no +1.
            self.divider_load_settle = self.pending_reload == TriggerReload::Enabling;
            self.divider.counter = (self.period.0) & 0x7FF;
            self.pending_reload = TriggerReload::Idle;
        } else if self.divider_load_settle {
            self.divider_load_settle = false;
        } else if self.divider.counter >= 0x7FF {
            // ch2_frst↑ (the overflow): dome captures the pre-advance duty step
            // and the divider reloads; the counter advances next cycle.
            let duty = self.waveform_and_initial_length.waveform() as usize;
            let latch = DUTY_TABLE[duty][self.wave_duty_position as usize] != 0;
            if latch != self.pwm_latch {
                self.pwm_latch = latch;
                self.output_dirty = true;
            }
            self.ch2_frst = true;
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
    /// next horu_512hz↑ sample so a same-step NR22 pace=0 write can
    /// clear `kyvo` and suppress the fire.
    pub fn tick_envelope_counter(&mut self) {
        self.envelope.tick_counter(
            self.volume_and_envelope.sweep_pace(),
            self.divider_load_settle,
        );
    }

    /// horu_512hz↑ edge (every fs step transition). Drains `kyvo` into
    /// the volume counter when `hafe` is asserted; otherwise consumes
    /// `kyvo` without firing (= dropped sample).
    pub fn sample_envelope_jopa(&mut self) {
        if self.envelope.sample_fire(
            self.volume_and_envelope.sweep_pace(),
            self.enabled.enabled,
            self.volume_and_envelope.direction(),
        ) {
            self.output_dirty = true;
        }
    }

    // DAC power = NRx2 upper five bits (FUTE).
    pub fn dac_enabled(&self) -> bool {
        self.volume_and_envelope.0 & 0xf8 != 0
    }

    /// The bit `dome` would capture at duty step `position`.
    pub fn duty_bit(&self, position: u8) -> bool {
        let duty = self.waveform_and_initial_length.waveform() as usize;
        DUTY_TABLE[duty][(position % 8) as usize] != 0
    }

    pub fn digital_sample(&self) -> u8 {
        if !self.enabled.enabled {
            return 0;
        }
        if self.pwm_latch {
            self.envelope.volume
        } else {
            0
        }
    }
}
