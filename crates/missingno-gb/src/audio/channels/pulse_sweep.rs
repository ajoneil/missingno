use super::{
    Enabled,
    envelope::Envelope,
    length::LengthCounter,
    registers::{
        PeriodDivider, PeriodHighAndControl, Signed11, VolumeAndEnvelope,
        WaveformAndInitialLength,
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
    Volume,
    PeriodSweep,
    PeriodLow,
    PeriodHighAndControl,
}

#[derive(Clone)]
pub struct PulseSweepChannel {
    pub enabled: Enabled,
    pub sweep: Sweep,
    pub waveform_and_initial_length: WaveformAndInitialLength,
    pub volume_and_envelope: VolumeAndEnvelope,
    pub length: LengthCounter<64>,
    pub period: Signed11,

    pub divider: PeriodDivider,
    pub wave_duty_position: u8,
    /// `duwo` PWM latch — captures the duty-pattern bit on each
    /// natural-overflow `ch1_frst ↑`; holds the emitted output
    /// between overflows.
    pub pwm_latch: bool,
    /// `ch1_restart` sync stage; non-zero between NR14 trigger write
    /// and the next ch1_1mhz↑ that applies the reload.
    pub pending_trigger_sync: u8,
    /// Set on the reload edge; the first count is suppressed so the
    /// divider DFFs settle out of load mode before counting resumes.
    pub divider_load_settle: bool,
    /// ch1_ld_sum holds the sweep counter across the trigger reload — a
    /// cate_128hz↓ in the load window is dropped. On CGB the hold spans one
    /// extra ch1_1mhz↑ beyond the divider's single-cycle settle (counts down
    /// per ch1_1mhz↑); 0 elsewhere, so DMG keeps the single-cycle settle.
    pub sweep_load_hold: u8,
    pub envelope: Envelope,
    pub shadow_frequency: u16,
    pub sweep_timer: u8,
    pub sweep_enabled: bool,
    pub sweep_negate_used: bool,
    /// `coze` (sweep-counter saturation). Set at cate_128hz↓ when the
    /// sweep counter reaches 0; sampled into BEXA on the next ajer↑.
    /// An NR10 pace=0 write in the intervening T-cycles clears it via
    /// the hafe async-reset path.
    pub coze: bool,
    /// This rise's cate↓ already ticked inside `tcycle` (the early wrap-
    /// coincident path); the frame sequencer's late pass must not repeat it.
    pub sweep_cate_taken: bool,
    /// `byra/caja/copa` — the sweep adder's shift-step counter, counting the
    /// steps left in the running calculation. The adder reads a *registered*
    /// snapshot of `shadow` and `shadow >> shift`, reloaded only when
    /// `ch1_ld_sum` pulses — which happens once this counter, loaded with
    /// `~shift`, saturates: `shift` steps, one per M-cycle (the `>> shift`
    /// operand is built a bit at a time). The overflow check fires at that
    /// reload. So a calc takes `shift` M-cycles. 0 = no calc running.
    pub sweep_calc_steps: u8,
    /// `ch1_restart` armed by a trigger; the adder calc reloads at the next
    /// ch1_1mhz↑ (the synced trigger edge), not on the NRx4 write.
    pub sweep_calc_restart: bool,
    /// `ch1_frst` overflow pulse — high for one `ch1_1mhz` cycle after an
    /// overflow. `duwo` captures the pre-advance duty on its rise (the
    /// overflow edge); the duty counter (`dajo`) clocks on its fall, one
    /// cycle later. So capture precedes advance.
    pub ch1_frst: bool,
    /// An input to `digital_sample()` / the mix may have changed.
    pub output_dirty: bool,
}

impl Default for PulseSweepChannel {
    fn default() -> Self {
        // Post-boot state at PC=0x0100 (boot ROM's Nintendo chime ran
        // CH1 to a known mid-period state with the envelope decayed).
        Self {
            enabled: Enabled {
                enabled: true,
                output_left: true,
                output_right: true,
            },
            sweep: Sweep(0x80),
            waveform_and_initial_length: WaveformAndInitialLength(0xbf),
            volume_and_envelope: VolumeAndEnvelope(0xf3),
            length: LengthCounter::default(),
            period: Signed11(0x7C1),
            divider: PeriodDivider { counter: 0x7F9 },
            wave_duty_position: 2,
            pwm_latch: false,
            pending_trigger_sync: 0,
            divider_load_settle: false,
            sweep_load_hold: 0,
            // chime decay ran to saturation; JEME latched
            envelope: Envelope {
                stopped: true,
                ..Envelope::default()
            },
            shadow_frequency: 0,
            sweep_timer: 0,
            sweep_enabled: false,
            sweep_negate_used: false,
            coze: false,
            sweep_cate_taken: false,
            sweep_calc_steps: 0,
            sweep_calc_restart: false,
            ch1_frst: false,
            output_dirty: true,
        }
    }
}

impl PulseSweepChannel {
    pub fn reset(&mut self) {
        let length_counter = self.length.counter; // DMG: length timers preserved on power-off
        *self = Self {
            enabled: Enabled::disabled(),
            sweep: Sweep(0),
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
            pending_trigger_sync: 0,
            divider_load_settle: false,
            sweep_load_hold: 0,
            envelope: Envelope::default(),
            shadow_frequency: 0,
            sweep_timer: 0,
            sweep_enabled: false,
            sweep_negate_used: false,
            coze: false,
            sweep_cate_taken: false,
            sweep_calc_steps: 0,
            sweep_calc_restart: false,
            ch1_frst: false,
            output_dirty: true,
        }
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::WaveformAndInitialLength => self.waveform_and_initial_length.0 | 0x3F,
            Register::Volume => self.volume_and_envelope.0,
            Register::PeriodSweep => self.sweep.0 | 0x80,
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
            Register::Volume => {
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
                // pace=0 raises jupu → hafe=0 → KOZY async-reset; any
                // armed kyvo is dropped before the next horu_512hz↑.
                if new_pace == 0 {
                    self.envelope.saturation_armed = false;
                }
                // Disabling the DAC immediately disables the channel
                if value & 0xf8 == 0 {
                    self.enabled.enabled = false;
                }
            }
            Register::PeriodSweep => {
                let old_negate = self.sweep.0 & 0b1000 != 0;
                self.sweep.0 = value;
                let new_negate = value & 0b1000 != 0;
                // Clearing negate bit after a negate calculation disables the channel
                if self.sweep_negate_used && old_negate && !new_negate {
                    self.enabled.enabled = false;
                }
                // pace=0 raises bury → hafe=0 → BEXA async-reset; any
                // armed coze — and a running adder calculation — is dropped
                // before ch1_ld_sum can latch an overflow into the stop latch.
                if self.sweep.pace() == 0 {
                    self.coze = false;
                    self.sweep_calc_steps = 0;
                    self.sweep_calc_restart = false;
                }
            }
            Register::PeriodLow => self.period.set_low8(value),
            Register::PeriodHighAndControl => {
                let ctrl = PeriodHighAndControl(value);
                self.period.set_high3(ctrl.period_high());

                // capy = NOR(cero, bufy_256hz, ff14_d6_n): length-enable
                // 0→1 rises capy (one extra length count) iff caru is low.
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
        // ch1_fdis (set by DAC-off / apu_reset, cleared by a trigger) gates the
        // divider toggle clock. Only the channel-enabling trigger — the one that
        // clears fdis 1→0 — freezes a load tick (the +1 first overflow); a
        // re-trigger of a running channel reloads with no +1. `2` flags the
        // enabling case to the reload arm.
        let was_running = self.enabled.enabled;
        self.enabled.enabled = true;
        self.length.trigger_reload();
        // Arm the ch1_restart sync: the reload applies at the next
        // ch1_1mhz↑, not on this write edge. A coincident natural
        // overflow on that wrap is suppressed (dyru async-resets
        // comy before cala can clock).
        self.pending_trigger_sync = if was_running { 1 } else { 2 };
        // ch1_restart pulls hafe low → KOZY reset → any prior kyvo
        // arm from the previous trigger window is dropped.
        self.envelope.trigger(
            self.volume_and_envelope.initial_volume(),
            self.volume_and_envelope.sweep_pace(),
        );

        // Initialize sweep
        self.sweep_negate_used = false;
        self.shadow_frequency = self.period.0;
        let pace = self.sweep.pace();
        self.sweep_timer = if pace != 0 { pace } else { 8 };
        self.sweep_enabled = pace != 0 || self.sweep.step() != 0;
        // ch1_restart resets BEXA: any prior coze arm is dropped.
        self.coze = false;
        // The adder calc restarts on ch1_restart — the *synced* trigger that
        // lands at the next ch1_1mhz↑ (where the divider reloads too), not on
        // this write edge. Armed here, loaded in `tcycle` at that wrap.
        self.sweep_calc_restart = true;

        // DAC check
        if self.volume_and_envelope.0 & 0xf8 == 0 {
            self.enabled.enabled = false;
        }
    }

    fn calculate_sweep_frequency(&mut self) -> u16 {
        let shifted = self.shadow_frequency >> self.sweep.step();
        match self.sweep.direction() {
            SweepDirection::Increasing => self.shadow_frequency.wrapping_add(shifted),
            SweepDirection::Decreasing => {
                self.sweep_negate_used = true;
                self.shadow_frequency.wrapping_sub(shifted)
            }
        }
    }

    pub fn tcycle(
        &mut self,
        apu_reset_n: bool,
        channel_clock_rose: bool,
        clock_phase_one: bool,
        wide_sweep_hold: bool,
        sweep_cate_due: bool,
    ) {
        // The shared channel clock's CALO↑ (ch1_1mhz↑) strobe.
        let calo_rose = channel_clock_rose;
        // cate↓ settles before the slot's wrap (measured sub-slot order:
        // cate +0.005, fire +0.25, wrap +0.52): tick it here so a wrap-
        // coincident arm's fire commits the period the wrap loads. A rise
        // with a trigger consume pending keeps the late (post-consume) cate
        // so the load-window hold still sees the settle; mid-count arms keep
        // the ajer↑ drain either way.
        if calo_rose {
            self.sweep_load_hold = self.sweep_load_hold.saturating_sub(1);
        }
        if sweep_cate_due
            && self.pending_trigger_sync == 0
            && calo_rose
            && self.divider.counter >= 0x7FF
        {
            self.sweep_cate_taken = true;
            self.tick_sweep_counter();
            self.sample_sweep_bexa(true);
        }
        // BEXA samples coze at the first ajer↑ of each M-cycle —
        // prescaler counter == 1 after the advance. Sample even when
        // the channel is disabled so a same-cycle re-trigger window
        // sees the cleared coze.
        if apu_reset_n && clock_phase_one {
            self.tick_sweep_calc();
            self.sample_sweep_bexa(false);
        }
        if !calo_rose || !self.enabled.enabled {
            return;
        }
        // ch1_restart latches the adder's ~shift step counter at this synced
        // ch1_1mhz↑ — the trigger's calc starts here, not on the NRx4 write.
        // ch1_ld_sum holds high one extra M-cycle while the counter loads
        // (the +1 the fire's continuing ld_sum cycle doesn't pay).
        if self.sweep_calc_restart {
            let shift = self.sweep.step();
            self.sweep_calc_steps = if shift != 0 { shift + 1 } else { 0 };
            self.sweep_calc_restart = false;
        }
        // ch1_frst↓ (one ch1_1mhz↑ after an overflow): the duty counter
        // (dajo) clocks on the fall, so the advance trails duwo's capture by
        // one cycle.
        if self.ch1_frst {
            self.wave_duty_position = (self.wave_duty_position + 1) % 8;
            self.ch1_frst = false;
        }
        // Prescaler wrapped (ch1_1mhz↑). Trigger reload and natural
        // overflow are mutually exclusive on the same edge — trigger
        // wins via dyru's async-reset of comy.
        if self.pending_trigger_sync != 0 {
            // Enabling trigger (2) freezes the load tick → +1 first overflow;
            // re-trigger (1) reloads with no +1.
            self.divider_load_settle = self.pending_trigger_sync == 2;
            // CGB holds the sweep counter one ch1_1mhz↑ longer than the
            // divider settle, so a cate_128hz↓ just past the reload is dropped.
            // CYMU = OR(BEXA, ch1_restart) drives the sweep counter's load
            // pins with no fdis input: every trigger reload — enabling or
            // re-trigger — opens the same 2-cycle CGB hold.
            self.sweep_load_hold = if wide_sweep_hold { 2 } else { 0 };
            self.divider.counter = (self.period.0) & 0x7FF;
            self.pending_trigger_sync = 0;
        } else if self.divider_load_settle {
            self.divider_load_settle = false;
        } else if self.divider.counter >= 0x7FF {
            // ch1_frst↑ (the overflow): duwo captures the pre-advance duty
            // step and the divider reloads; the counter advances next cycle.
            let duty = self.waveform_and_initial_length.waveform() as usize;
            let latch = DUTY_TABLE[duty][self.wave_duty_position as usize] != 0;
            if latch != self.pwm_latch {
                self.pwm_latch = latch;
                self.output_dirty = true;
            }
            self.ch1_frst = true;
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
    /// next horu_512hz↑ sample so a same-step NR12 pace=0 write can
    /// clear `kyvo` and suppress the fire (CH1 mirror of CH2).
    pub fn tick_envelope_counter(&mut self) {
        self.envelope.tick_counter(
            self.volume_and_envelope.sweep_pace(),
            self.divider_load_settle,
        );
    }

    /// horu_512hz↑ edge (every fs step transition). Drains `kyvo` into
    /// the volume counter when `hafe` is asserted; otherwise consumes
    /// `kyvo` without firing.
    pub fn sample_envelope_jopa(&mut self) {
        if self.envelope.sample_fire(
            self.volume_and_envelope.sweep_pace(),
            self.enabled.enabled,
            self.volume_and_envelope.direction(),
        ) {
            self.output_dirty = true;
        }
    }

    /// cate_128hz↓ edge (fs steps 2 and 6). Decrements the sweep
    /// counter; when it reaches 0 it reloads to pace and arms `coze`
    /// for sampling by the next ajer↑. The actual overflow check /
    /// period update / channel-disable are deferred to BEXA's sample
    /// so an NR10 pace=0 write in the intervening T-cycle window can
    /// suppress the fire via the bury async-reset path.
    pub fn tick_sweep_counter(&mut self) {
        // dmg_tffnl holds the counter while the divider load window is open —
        // a cate_128hz↓ inside the window is skipped. `sweep_load_hold` carries
        // the CGB extra cycle (0 on DMG, leaving the single-cycle settle).
        if self.divider_load_settle || self.sweep_load_hold > 0 {
            return;
        }
        if !self.sweep_enabled {
            return;
        }
        if self.sweep_timer > 0 {
            self.sweep_timer -= 1;
        }
        if self.sweep_timer == 0 {
            let pace = self.sweep.pace();
            self.sweep_timer = if pace != 0 { pace } else { 8 };
            if pace != 0 {
                self.coze = true;
            }
        }
    }

    /// ajer↑ edge (CALO|AJER prescaler counter = 1 = first ajer↑ in
    /// the M-cycle). Drains `coze`: runs the overflow check and the
    /// shadow / period update; if the overflow result would set the
    /// channel-disable, do so. Cleared without firing when pace=0.
    pub fn take_sweep_cate(&mut self) -> bool {
        let taken = self.sweep_cate_taken;
        self.sweep_cate_taken = false;
        taken
    }

    pub fn sample_sweep_bexa(&mut self, early: bool) {
        if !self.coze {
            return;
        }
        if self.sweep.pace() == 0 {
            self.coze = false;
            return;
        }
        let new_frequency = self.calculate_sweep_frequency();
        if new_frequency > 2047 {
            // A calc1 overflow disables at the ajer↑ (presc==1) drain — the
            // ch1_ld_sum resolution edge — not at an early wrap-coincident
            // cate. The early path exists only to commit a period for the
            // coincident wrap, so on an overflow it leaves coze for the drain.
            if early {
                return;
            }
            self.coze = false;
            self.enabled.enabled = false;
            self.output_dirty = true;
        } else {
            self.coze = false;
            if self.sweep.step() != 0 {
                // Commit calc1, then restart the adder calculation: the recheck
                // on the committed period overflows `shift` M-cycles on
                // (ch1_ld_sum).
                self.shadow_frequency = new_frequency;
                self.period.0 = new_frequency;
                self.sweep_calc_steps = self.sweep.step();
            }
        }
    }

    /// The sweep adder's `~shift` step counter, advanced one step per M-cycle.
    /// When it saturates, `ch1_ld_sum` re-snapshots `shadow` / `shadow >> shift`
    /// into the adder operands; if the result overflows (direction = add), the
    /// stop latch (`cyto`) clears. So an overflow disables the channel `shift`
    /// M-cycles after the fire/trigger that started the calculation.
    pub fn tick_sweep_calc(&mut self) {
        if self.sweep_calc_steps == 0 {
            return;
        }
        self.sweep_calc_steps -= 1;
        if self.sweep_calc_steps == 0 && self.calculate_sweep_frequency() > 2047 {
            self.enabled.enabled = false;
            self.output_dirty = true;
        }
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

pub enum SweepDirection {
    Increasing,
    Decreasing,
}

#[derive(Clone)]
pub struct Sweep(pub u8);

impl Sweep {
    pub fn pace(&self) -> u8 {
        (self.0 & 0b0111_0000) >> 4
    }

    pub fn direction(&self) -> SweepDirection {
        if self.0 & 0b1000 != 0 {
            SweepDirection::Decreasing
        } else {
            SweepDirection::Increasing
        }
    }

    pub fn step(&self) -> u8 {
        self.0 & 0b111
    }
}
