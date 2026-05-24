use super::{
    Enabled,
    registers::{
        EnvelopeDirection, PeriodDivider, PeriodHighAndControl, Prescaler, Signed11,
        VolumeAndEnvelope, WaveformAndInitialLength,
    },
};

const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
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
    pub length_enabled: bool,
    pub period: Signed11,

    pub prescaler: Prescaler,
    pub divider: PeriodDivider,
    pub wave_duty_position: u8,
    /// DUWO — duty PWM latch captured on each natural-overflow `ch1_frst↑`.
    /// Holds the channel's emitted bit between overflows. Spec §14.6.1.
    pub pwm_latch: bool,
    /// Trigger reload synchroniser: NR14 writes set this to 1, the next
    /// prescaler wrap promotes it to 2, the wrap after that applies the
    /// reload. Models the `ch1_restart` DFF that captures at the next
    /// `ch1_1mhz↑` after the NR14 write. Spec §14.5 / §14.5.1.
    pub pending_trigger_sync: u8,
    /// Load-mode settle latency per §14.5.1.1 — after the trigger
    /// reload, the divider DFFs need one full `ch1_1mhz` cycle to
    /// settle out of load mode before they begin counting. Set on
    /// the reload wrap; cleared (without counting) on the next wrap;
    /// natural counting resumes on the wrap after.
    pub divider_load_settle: bool,
    pub current_volume: u8,
    pub envelope_timer: u8,
    pub length_counter: u16,
    pub shadow_frequency: u16,
    pub sweep_timer: u8,
    pub sweep_enabled: bool,
    pub sweep_negate_used: bool,
}

impl Default for PulseSweepChannel {
    fn default() -> Self {
        // Post-boot state at PC=0x0100, per spec §11.5 (FST-anchored via
        // dmg-sim with the production DMG boot ROM). The boot ROM's
        // Nintendo chime triggered CH1, ran the envelope to 0, and left
        // the period divider mid-period.
        Self {
            enabled: Enabled {
                enabled: true, // ch1_fdis = 0 (channel running)
                output_left: true,
                output_right: true,
            },
            sweep: Sweep(0x80),
            waveform_and_initial_length: WaveformAndInitialLength(0xbf),
            volume_and_envelope: VolumeAndEnvelope(0xf3),
            length_enabled: false,
            period: Signed11(0x7C1), // acc_d at handoff (= {NR14[2:0], NR13[7:0]} = {7, 0xC1})

            prescaler: Prescaler { counter: 1 }, // (calo, ajer) = (0, 1)
            divider: PeriodDivider { counter: 0x7F9 }, // 6 ticks below natural overflow
            wave_duty_position: 2,               // duty step counter (dape, eros, esut) = 010
            pwm_latch: false, // duwo: boot ROM's last overflow captured pattern[1] = 0 (50%)
            pending_trigger_sync: 0,
            divider_load_settle: false,
            current_volume: 0, // envelope decayed
            envelope_timer: 0,
            length_counter: 0,
            shadow_frequency: 0,
            sweep_timer: 0,
            sweep_enabled: false,
            sweep_negate_used: false,
        }
    }
}

impl PulseSweepChannel {
    pub fn reset(&mut self) {
        let length_counter = self.length_counter; // DMG: length timers preserved on power-off
        *self = Self {
            enabled: Enabled::disabled(),
            sweep: Sweep(0),
            waveform_and_initial_length: WaveformAndInitialLength(0),
            volume_and_envelope: VolumeAndEnvelope(0),
            length_enabled: false,
            period: (0).into(),

            prescaler: Prescaler::default(),
            divider: PeriodDivider::default(),
            wave_duty_position: 0,
            pwm_latch: false,
            pending_trigger_sync: 0,
            divider_load_settle: false,
            current_volume: 0,
            envelope_timer: 0,
            length_counter,
            shadow_frequency: 0,
            sweep_timer: 0,
            sweep_enabled: false,
            sweep_negate_used: false,
        }
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::WaveformAndInitialLength => self.waveform_and_initial_length.0 | 0x3F,
            Register::Volume => self.volume_and_envelope.0,
            Register::PeriodSweep => self.sweep.0 | 0x80,
            Register::PeriodLow => 0xff,
            Register::PeriodHighAndControl => PeriodHighAndControl::read(self.length_enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8, frame_sequencer_step: u8) {
        match register {
            Register::WaveformAndInitialLength => {
                self.waveform_and_initial_length = WaveformAndInitialLength(value);
                self.length_counter = 64 - self.waveform_and_initial_length.initial_length() as u16;
            }
            Register::Volume => {
                self.volume_and_envelope = VolumeAndEnvelope(value);
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
            }
            Register::PeriodLow => self.period.set_low8(value),
            Register::PeriodHighAndControl => {
                let ctrl = PeriodHighAndControl(value);
                self.period.set_high3(ctrl.period_high());

                // Extra length clocking on NRx4 write
                let next_step_clocks_length = matches!(frame_sequencer_step, 0 | 2 | 4 | 6);
                let was_length_enabled = self.length_enabled;
                self.length_enabled = ctrl.enable_length();

                if !next_step_clocks_length
                    && !was_length_enabled
                    && self.length_enabled
                    && self.length_counter > 0
                {
                    self.length_counter -= 1;
                    if self.length_counter == 0 && !ctrl.trigger() {
                        self.enabled.enabled = false;
                    }
                }

                if ctrl.trigger() {
                    self.trigger();
                    if !next_step_clocks_length && self.length_enabled && self.length_counter == 64
                    {
                        self.length_counter = 63;
                    }
                }
            }
        }
    }

    pub fn trigger(&mut self) {
        self.enabled.enabled = true;
        if self.length_counter == 0 {
            self.length_counter = 64;
        }
        // ch1_restart synchroniser: don't reload the divider directly.
        // The reload happens at the next prescaler wrap (= next chN_1mhz↑
        // edge). If a natural overflow lands on the same wrap, it fires
        // first (spec §14.5.1 — pos_1 vs pos_2 asymmetry).
        self.pending_trigger_sync = 1;
        self.current_volume = self.volume_and_envelope.initial_volume();
        self.envelope_timer = self.volume_and_envelope.sweep_pace();

        // Initialize sweep
        self.sweep_negate_used = false;
        self.shadow_frequency = self.period.0;
        let pace = self.sweep.pace();
        self.sweep_timer = if pace != 0 { pace } else { 8 };
        self.sweep_enabled = pace != 0 || self.sweep.step() != 0;

        // If step is non-zero, do overflow check
        if self.sweep.step() != 0 && self.calculate_sweep_frequency() > 2047 {
            self.enabled.enabled = false;
        }

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

    pub fn tcycle(&mut self, apu_reset_n: bool) {
        if !self.prescaler.tcycle(apu_reset_n) || !self.enabled.enabled {
            return;
        }
        // Prescaler wrapped (ch1_1mhz↑). Per spec §14.5.1, the trigger
        // synchroniser ch1_restart and the divider reload both resolve
        // on this same edge — dyru async-resets COMY before cala can
        // clock, so a coincident natural overflow is suppressed.
        if self.pending_trigger_sync != 0 {
            self.divider.counter = (self.period.0) & 0x7FF;
            self.pending_trigger_sync = 0;
            // §14.5.1.1 load-mode settle: first count after a trigger
            // is on the SECOND ch1_1mhz↑ after the reload, not the first.
            self.divider_load_settle = true;
        } else if self.divider_load_settle {
            // Skip the count cycle so the divider DFFs settle out of
            // level-sensitive load mode before counting resumes.
            self.divider_load_settle = false;
        } else if self.divider.counter >= 0x7FF {
            // Natural overflow: capture duwo at pre-advance counter,
            // then advance the duty step counter and reload divider.
            let duty = self.waveform_and_initial_length.waveform() as usize;
            self.pwm_latch = DUTY_TABLE[duty][self.wave_duty_position as usize] != 0;
            self.wave_duty_position = (self.wave_duty_position + 1) % 8;
            self.divider.counter = (self.period.0) & 0x7FF;
        } else {
            self.divider.counter += 1;
        }
    }

    pub fn tick_length(&mut self) {
        if self.length_enabled && self.length_counter > 0 {
            self.length_counter -= 1;
            if self.length_counter == 0 {
                self.enabled.enabled = false;
            }
        }
    }

    pub fn tick_envelope(&mut self) {
        let pace = self.volume_and_envelope.sweep_pace();
        if pace == 0 {
            return;
        }

        if self.envelope_timer > 0 {
            self.envelope_timer -= 1;
        }
        if self.envelope_timer == 0 {
            self.envelope_timer = pace;
            match self.volume_and_envelope.direction() {
                EnvelopeDirection::Increase => {
                    if self.current_volume < 15 {
                        self.current_volume += 1;
                    }
                }
                EnvelopeDirection::Decrease => {
                    if self.current_volume > 0 {
                        self.current_volume -= 1;
                    }
                }
            }
        }
    }

    pub fn tick_sweep(&mut self) {
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
                let new_frequency = self.calculate_sweep_frequency();
                if new_frequency > 2047 {
                    self.enabled.enabled = false;
                } else if self.sweep.step() != 0 {
                    self.shadow_frequency = new_frequency;
                    self.period.0 = new_frequency;

                    // Overflow check again with new frequency
                    if self.calculate_sweep_frequency() > 2047 {
                        self.enabled.enabled = false;
                    }
                }
            }
        }
    }

    pub fn sample(&self) -> f32 {
        if !self.enabled.enabled {
            return 0.0;
        }
        // DUWO holds the duty pattern bit captured at the last natural
        // overflow (spec §14.6.1). The combinational chN_pwm output that
        // reflects the current counter position is NOT what feeds the
        // DAC — only the latched value does.
        let output = if self.pwm_latch { 1u8 } else { 0 };
        output as f32 * self.current_volume as f32 / 15.0
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
