use super::pulse::PulseChannel;

/// The pulse channels' sweep seam. CH1's silicon is CH2's plus the period
/// sweep, so the shared channel carries the unit as a type parameter: [`Sweep`]
/// on CH1, [`NoSweep`] on CH2, whose hooks fold away in its monomorphization.
pub trait SweepUnit: Clone + Default {
    /// chN_restart: reload the shadow period and the sweep counter, and arm the
    /// adder calculation for the synced trigger edge.
    fn on_trigger(&mut self, _period: u16) {}

    /// The synced chN_1mhz↑ that applies a trigger.
    fn on_channel_clock(&mut self) {}

    /// The divider's trigger reload. `wide` widens the load window by one
    /// chN_1mhz↑ past the divider settle.
    fn on_trigger_reload(&mut self, _wide: bool) {}
}

/// CH2, which carries no sweep unit.
#[derive(Clone, Copy, Default)]
pub struct NoSweep;

impl SweepUnit for NoSweep {}

pub enum SweepDirection {
    Increasing,
    Decreasing,
}

/// NR10.
#[derive(Clone, Copy, Default)]
pub struct PeriodSweep(pub u8);

impl PeriodSweep {
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

/// CH1's period sweep: NR10, the shadow period the adder works on, and the
/// counters that pace it.
#[derive(Clone, Default)]
pub struct Sweep {
    pub register: PeriodSweep,
    pub shadow_frequency: u16,
    pub timer: u8,
    pub enabled: bool,
    pub negate_used: bool,
    /// COZE (sweep-counter saturation). Set at cate_128hz↓ when the
    /// sweep counter reaches 0; sampled into BEXA on the next ajer↑.
    /// An NR10 pace=0 write in the intervening T-cycles clears it via
    /// the hafe async-reset path.
    pub counter_at_max: bool,
    /// This rise's cate↓ already ticked inside `tcycle` (the early wrap-
    /// coincident path); the frame sequencer's late pass must not repeat it.
    pub cate_taken: bool,
    /// `byra/caja/copa` — the sweep adder's shift-step counter, counting the
    /// steps left in the running calculation. The adder reads a *registered*
    /// snapshot of `shadow` and `shadow >> shift`, reloaded only when
    /// `ch1_ld_sum` pulses — which happens once this counter, loaded with
    /// `~shift`, saturates: `shift` steps, one per M-cycle (the `>> shift`
    /// operand is built a bit at a time). The overflow check fires at that
    /// reload. So a calc takes `shift` M-cycles. 0 = no calc running.
    pub calc_steps: u8,
    /// `ch1_restart` armed by a trigger; the adder calc reloads at the next
    /// ch1_1mhz↑ (the synced trigger edge), not on the NRx4 write.
    pub calc_restart: bool,
    /// ch1_ld_sum holds the sweep counter across the trigger reload — a
    /// cate_128hz↓ in the load window is dropped. On CGB the hold spans one
    /// extra ch1_1mhz↑ beyond the divider's single-cycle settle (counts down
    /// per ch1_1mhz↑); 0 elsewhere, so DMG keeps the single-cycle settle.
    pub load_hold: u8,
}

impl SweepUnit for Sweep {
    fn on_trigger(&mut self, period: u16) {
        self.negate_used = false;
        self.shadow_frequency = period;
        let pace = self.register.pace();
        self.timer = if pace != 0 { pace } else { 8 };
        self.enabled = pace != 0 || self.register.step() != 0;
        // ch1_restart resets BEXA: any prior coze arm is dropped.
        self.counter_at_max = false;
        // The adder calc restarts on ch1_restart — the *synced* trigger that
        // lands at the next ch1_1mhz↑ (where the divider reloads too), not on
        // the NRx4 write edge. Armed here, loaded at that wrap.
        self.calc_restart = true;
    }

    fn on_channel_clock(&mut self) {
        // ch1_restart latches the adder's ~shift step counter at this synced
        // ch1_1mhz↑. ch1_ld_sum holds high one extra M-cycle while the counter
        // loads (the +1 the fire's continuing ld_sum cycle doesn't pay).
        if self.calc_restart {
            let shift = self.register.step();
            self.calc_steps = if shift != 0 { shift + 1 } else { 0 };
            self.calc_restart = false;
        }
    }

    fn on_trigger_reload(&mut self, wide: bool) {
        // CYMU = OR(BEXA, ch1_restart) drives the sweep counter's load pins with
        // no fdis input: every trigger reload — enabling or re-trigger — opens
        // the same hold, two ch1_1mhz↑ wide on CGB.
        self.load_hold = if wide { 2 } else { 0 };
    }
}

impl PulseChannel<Sweep> {
    pub fn read_period_sweep(&self) -> u8 {
        self.sweep.register.0 | 0x80
    }

    pub fn write_period_sweep(&mut self, value: u8) {
        self.output_dirty = true;
        let old_negate = self.sweep.register.0 & 0b1000 != 0;
        self.sweep.register.0 = value;
        let new_negate = value & 0b1000 != 0;
        // Clearing negate bit after a negate calculation disables the channel
        if self.sweep.negate_used && old_negate && !new_negate {
            self.enabled.enabled = false;
        }
        // pace=0 raises bury → hafe=0 → BEXA async-reset; any
        // armed coze — and a running adder calculation — is dropped
        // before ch1_ld_sum can latch an overflow into the stop latch.
        if self.sweep.register.pace() == 0 {
            self.sweep.counter_at_max = false;
            self.sweep.calc_steps = 0;
            self.sweep.calc_restart = false;
        }
    }

    /// One master-clock rise. `channel_clock_rose` is the shared CALO↑
    /// (ch1_1mhz↑) strobe, low while apu_reset holds the clock.
    pub fn tcycle(
        &mut self,
        apu_reset_n: bool,
        channel_clock_rose: bool,
        clock_phase_one: bool,
        wide_sweep_hold: bool,
        sweep_cate_due: bool,
    ) {
        // cate↓ settles before the slot's wrap (measured sub-slot order:
        // cate +0.005, fire +0.25, wrap +0.52): tick it here so a wrap-
        // coincident arm's fire commits the period the wrap loads. A rise
        // with a trigger consume pending keeps the late (post-consume) cate
        // so the load-window hold still sees the settle; mid-count arms keep
        // the ajer↑ drain either way.
        if channel_clock_rose {
            self.sweep.load_hold = self.sweep.load_hold.saturating_sub(1);
        }
        if sweep_cate_due
            && self.pending_reload == super::TriggerReload::Idle
            && channel_clock_rose
            && self.divider.counter >= 0x7FF
        {
            self.sweep.cate_taken = true;
            self.tick_sweep_counter();
            self.sample_sweep_fire(true);
        }
        // BEXA samples coze at the first ajer↑ of each M-cycle —
        // prescaler counter == 1 after the advance. Sample even when
        // the channel is disabled so a same-cycle re-trigger window
        // sees the cleared coze.
        if apu_reset_n && clock_phase_one {
            self.tick_sweep_calc();
            self.sample_sweep_fire(false);
        }
        self.tick_divider(channel_clock_rose, wide_sweep_hold);
    }

    fn calculate_sweep_frequency(&mut self) -> u16 {
        let shadow = self.sweep.shadow_frequency;
        let shifted = shadow >> self.sweep.register.step();
        match self.sweep.register.direction() {
            SweepDirection::Increasing => shadow.wrapping_add(shifted),
            SweepDirection::Decreasing => {
                self.sweep.negate_used = true;
                shadow.wrapping_sub(shifted)
            }
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
        // a cate_128hz↓ inside the window is skipped. `load_hold` carries the
        // CGB extra cycle (0 on DMG, leaving the single-cycle settle).
        if self.divider_load_settle || self.sweep.load_hold > 0 {
            return;
        }
        if !self.sweep.enabled {
            return;
        }
        if self.sweep.timer > 0 {
            self.sweep.timer -= 1;
        }
        if self.sweep.timer == 0 {
            let pace = self.sweep.register.pace();
            self.sweep.timer = if pace != 0 { pace } else { 8 };
            if pace != 0 {
                self.sweep.counter_at_max = true;
            }
        }
    }

    /// Whether the early wrap-coincident path already delivered this rise's
    /// cate↓, so the frame sequencer's late pass skips it.
    pub fn take_sweep_cate(&mut self) -> bool {
        let taken = self.sweep.cate_taken;
        self.sweep.cate_taken = false;
        taken
    }

    /// BEXA: sample the armed COZE into the sweep-fire latch. Drains `coze`:
    /// runs the overflow check and the shadow / period update; if the overflow
    /// result would set the channel-disable, do so. Cleared without firing when
    /// pace=0.
    pub fn sample_sweep_fire(&mut self, early: bool) {
        if !self.sweep.counter_at_max {
            return;
        }
        if self.sweep.register.pace() == 0 {
            self.sweep.counter_at_max = false;
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
            self.sweep.counter_at_max = false;
            self.enabled.enabled = false;
            self.output_dirty = true;
        } else {
            self.sweep.counter_at_max = false;
            if self.sweep.register.step() != 0 {
                // Commit calc1, then restart the adder calculation: the recheck
                // on the committed period overflows `shift` M-cycles on
                // (ch1_ld_sum).
                self.sweep.shadow_frequency = new_frequency;
                self.period.0 = new_frequency;
                self.sweep.calc_steps = self.sweep.register.step();
            }
        }
    }

    /// The sweep adder's `~shift` step counter, advanced one step per M-cycle.
    /// When it saturates, `ch1_ld_sum` re-snapshots `shadow` / `shadow >> shift`
    /// into the adder operands; if the result overflows (direction = add), the
    /// stop latch (`cyto`) clears. So an overflow disables the channel `shift`
    /// M-cycles after the fire/trigger that started the calculation.
    pub fn tick_sweep_calc(&mut self) {
        if self.sweep.calc_steps == 0 {
            return;
        }
        self.sweep.calc_steps -= 1;
        if self.sweep.calc_steps == 0 && self.calculate_sweep_frequency() > 2047 {
            self.enabled.enabled = false;
            self.output_dirty = true;
        }
    }
}
