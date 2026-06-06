use super::{
    Enabled,
    registers::{
        EnvelopeDirection, PeriodDivider, PeriodHighAndControl, Prescaler, Signed11,
        VolumeAndEnvelope, WaveformAndInitialLength,
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
pub struct PulseChannel {
    pub enabled: Enabled,
    pub waveform_and_initial_length: WaveformAndInitialLength,
    pub volume_and_envelope: VolumeAndEnvelope,
    pub length_enabled: bool,
    pub period: Signed11,

    pub prescaler: Prescaler,
    pub divider: PeriodDivider,
    pub wave_duty_position: u8,
    /// `dome` PWM latch (CH2 mirror of CH1's `duwo`).
    pub pwm_latch: bool,
    /// `ch2_restart` sync stage; non-zero between NR24 trigger write
    /// and the next ch2_1mhz↑ that applies the reload.
    pub pending_trigger_sync: u8,
    /// Set on the reload edge; the first count is suppressed so the
    /// divider DFFs settle out of load mode (CH1/CH2 mirror).
    pub divider_load_settle: bool,
    pub current_volume: u8,
    pub envelope_timer: u8,
    /// `kyvo` (envelope-counter saturation). Set at kene↓ when the
    /// envelope counter reaches 0. Sampled into JOPA on the next
    /// horu_512hz↑; that's the deferred edge that actually advances
    /// the volume counter on hardware.
    pub kyvo: bool,
    pub length_counter: u16,
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
            length_enabled: false,
            period: Signed11(0), // CH2 NR23/NR24 never written by boot ROM; acc_d = 0

            prescaler: Prescaler::default(),
            divider: PeriodDivider::default(),
            wave_duty_position: 0,
            pwm_latch: false,
            pending_trigger_sync: 0,
            divider_load_settle: false,
            current_volume: 0,
            envelope_timer: 0,
            kyvo: false,
            length_counter: 0,
        }
    }
}

impl PulseChannel {
    pub fn reset(&mut self) {
        let length_counter = self.length_counter; // DMG: length timers preserved on power-off
        *self = Self {
            enabled: Enabled::disabled(),
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
            kyvo: false,
            length_counter,
        };
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::WaveformAndInitialLength => self.waveform_and_initial_length.0 | 0x3F,
            Register::VolumeAndEnvelope => self.volume_and_envelope.0,
            Register::PeriodLow => 0xff,
            Register::PeriodHighAndControl => PeriodHighAndControl::read(self.length_enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8, caru_low: bool) {
        match register {
            Register::WaveformAndInitialLength => {
                self.waveform_and_initial_length = WaveformAndInitialLength(value);
                self.length_counter = 64 - self.waveform_and_initial_length.initial_length() as u16;
            }
            Register::VolumeAndEnvelope => {
                self.volume_and_envelope = VolumeAndEnvelope(value);
                // pace=0 raises jupu → hafe=0 → JOPA async-reset; any
                // armed kyvo is dropped before the next horu_512hz↑.
                if self.volume_and_envelope.sweep_pace() == 0 {
                    self.kyvo = false;
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
                let was_length_enabled = self.length_enabled;
                self.length_enabled = ctrl.enable_length();

                if caru_low && !was_length_enabled && self.length_enabled && self.length_counter > 0
                {
                    self.length_counter -= 1;
                    if self.length_counter == 0 && !ctrl.trigger() {
                        self.enabled.enabled = false;
                    }
                }

                if ctrl.trigger() {
                    self.trigger();
                    if caru_low && self.length_enabled && self.length_counter == 64 {
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
        // Arm the ch2_restart sync: the reload applies at the next
        // ch2_1mhz↑, not on this write edge.
        self.pending_trigger_sync = 1;
        self.current_volume = self.volume_and_envelope.initial_volume();
        self.envelope_timer = self.volume_and_envelope.sweep_pace();
        // ch2_restart pulls hafe low → JOPA reset → any prior kyvo
        // arm from the previous trigger window is dropped.
        self.kyvo = false;

        // DAC check: if upper 5 bits of volume register are 0, channel is disabled
        if self.volume_and_envelope.0 & 0xf8 == 0 {
            self.enabled.enabled = false;
        }
    }

    pub fn tcycle(&mut self, apu_reset_n: bool) {
        if !self.prescaler.tcycle(apu_reset_n) || !self.enabled.enabled {
            return;
        }
        if self.pending_trigger_sync != 0 {
            self.divider.counter = (self.period.0) & 0x7FF;
            self.pending_trigger_sync = 0;
            // First post-reload tick is consumed by load-mode settle.
            self.divider_load_settle = true;
        } else if self.divider_load_settle {
            self.divider_load_settle = false;
        } else if self.divider.counter >= 0x7FF {
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

    /// kene↓ edge (fs step 7→0). Advances the envelope counter and
    /// arms `kyvo` on saturation; the volume update is deferred to the
    /// next horu_512hz↑ sample so a same-step NR22 pace=0 write can
    /// clear `kyvo` and suppress the fire.
    pub fn tick_envelope_counter(&mut self) {
        // dmg_tffnl holds the counter (ignores tclk_n) while the divider load
        // window is open — a kene↓ inside the window is skipped, slipping the
        // first fire one 64 Hz period.
        if self.divider_load_settle {
            return;
        }
        let pace = self.volume_and_envelope.sweep_pace();
        if pace == 0 {
            return;
        }
        if self.envelope_timer > 0 {
            self.envelope_timer -= 1;
        }
        if self.envelope_timer == 0 {
            self.envelope_timer = pace;
            self.kyvo = true;
        }
    }

    /// horu_512hz↑ edge (every fs step transition). Drains `kyvo` into
    /// the volume counter when `hafe` is asserted; otherwise consumes
    /// `kyvo` without firing (= dropped sample).
    pub fn sample_envelope_jopa(&mut self) {
        if !self.kyvo {
            return;
        }
        self.kyvo = false;
        let pace = self.volume_and_envelope.sweep_pace();
        if pace == 0 || !self.enabled.enabled {
            return;
        }
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

    pub fn digital_sample(&self) -> u8 {
        if !self.enabled.enabled {
            return 0;
        }
        if self.pwm_latch {
            self.current_volume
        } else {
            0
        }
    }
}
