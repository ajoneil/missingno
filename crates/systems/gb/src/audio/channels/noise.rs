use super::{
    Enabled,
    envelope::Envelope,
    length::LengthCounter,
    registers::{Prescaler, VolumeAndEnvelope},
};

#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    LengthTimer,
    VolumeAndEnvelope,
    FrequencyAndRandomness,
    Control,
}

#[derive(Clone)]
pub struct NoiseChannel {
    pub enabled: Enabled,
    pub volume_and_envelope: VolumeAndEnvelope,
    pub length: LengthCounter<64>,
    pub frequency_and_randomness: FrequencyAndRandomness,

    /// 14-bit shift divider (CEXO…ESEP). The NR43 shift code combinationally
    /// taps bit `shift`; the LFSR shifts on its rising edge. Free-running once
    /// started — a mid-run NR43 write re-taps it without disturbing its phase.
    pub divider: u16,
    /// T-cycles to the next `ch4_1mhz` tick opportunity (period 4 T); the divider
    /// advances on it only when `gary` gates it through.
    pub divider_subcounter: u16,
    /// Divisor prescaler (JYCO/JYRE/JYFU): a 3-bit up-counter that `huce` loads
    /// with `~NR43[2:0]` and that otherwise counts to terminal 7 (HYNO) on the
    /// 8 T `kanu` grid. Code 0 loads 7 = terminal, so it is pinned and `gary`
    /// stays high (divider at the full 4 T rate).
    pub prescaler: u8,
    /// Divider-clock enable (GARY): the latched prescaler terminal. Gates
    /// `noise_counter_clk = ch4_1mhz AND gary`, and drives `huce = ch4_restart OR
    /// gary` — so a mid-run code change is taken at the next terminal, immediately
    /// while on code 0 (huce continuous).
    pub gary: bool,
    /// Cold synchroniser hold: the divider is frozen for this many T after a
    /// trigger so the first tap lands at `sync_delay + period/2` = the cold-load.
    pub sync_delay: u16,
    /// Previous tapped-bit level, for rising-edge detection.
    pub prev_tap: bool,
    /// `ch4_1mhz` /4 prescaler (BAVU), `t_index`-anchored — same cell as the
    /// pulse channels' chN_1mhz divider; drives the hama half-phase.
    pub mhz_prescaler: Prescaler,
    /// Hama half-phase (`jeso`, ÷2 of `ch4_1mhz`). The code ≥ 1 cold-load snaps
    /// to the hama grid by this; flips each `ch4_1mhz↑`, reset only on apu-off.
    pub jeso: bool,
    /// KEY1 double-speed, latched each tcycle. At double speed the cold-load
    /// gains the prescaler-terminal-relative hold read by `trigger()`.
    pub double_speed: bool,
    /// Set by a re-trigger of a running channel: its first divider expiry is
    /// swallowed so the first LFSR shift lands one sample later than a cold trigger.
    pub skip_first_clock: bool,
    pub lfsr: u16,
    pub envelope: Envelope,
    /// An input to `digital_sample()` / the mix may have changed.
    pub output_dirty: bool,
}

impl Default for NoiseChannel {
    fn default() -> Self {
        Self {
            enabled: Enabled {
                enabled: false,
                output_left: true,
                output_right: false,
            },
            volume_and_envelope: VolumeAndEnvelope(0),
            length: LengthCounter::default(),
            frequency_and_randomness: FrequencyAndRandomness(0),

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
            lfsr: 0x7fff,
            envelope: Envelope::default(),
            output_dirty: true,
        }
    }
}

impl NoiseChannel {
    pub fn reset(&mut self) {
        let length_counter = self.length.counter; // DMG: NR41 length timer preserved on power-off
        self.enabled = Enabled::disabled();
        self.volume_and_envelope = VolumeAndEnvelope(0);
        self.length = LengthCounter {
            enabled: false,
            counter: length_counter,
        };
        self.frequency_and_randomness = FrequencyAndRandomness(0);

        self.divider = 0;
        self.divider_subcounter = 0;
        self.prescaler = 0;
        self.gary = false;
        self.sync_delay = 0;
        self.prev_tap = false;
        self.mhz_prescaler = Prescaler::default();
        self.jeso = false;
        self.skip_first_clock = false;
        self.lfsr = 0x7fff;
        self.envelope = Envelope::default();
        self.output_dirty = true;
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::LengthTimer => 0xff,
            Register::VolumeAndEnvelope => self.volume_and_envelope.0,
            Register::FrequencyAndRandomness => self.frequency_and_randomness.0,
            Register::Control => Control::read(self.length.enabled),
        }
    }

    pub fn write_register(
        &mut self,
        register: Register,
        value: u8,
        caru_low: bool,
        grid_anchor: bool,
    ) {
        self.output_dirty = true;
        match register {
            Register::LengthTimer => {
                self.length.load((value & 0x3f) as u16);
            }
            Register::VolumeAndEnvelope => {
                // Write-strobe transient (CH4 mirror of CH1/CH2): one +1
                // volume clock iff the old pace was 0, free 4-bit wrap.
                self.envelope
                    .zombie_bump(self.volume_and_envelope.sweep_pace());
                self.volume_and_envelope = VolumeAndEnvelope(value);
                // pace=0 → JOPA async-reset; any armed kyvo is dropped before horu↑.
                if self.volume_and_envelope.sweep_pace() == 0 {
                    self.envelope.saturation_armed = false;
                }
                // Disabling the DAC immediately disables the channel
                if value & 0xf8 == 0 {
                    self.enabled.enabled = false;
                }
            }
            Register::FrequencyAndRandomness => {
                let old_shift = self.frequency_and_randomness.clock_shift();
                let old_code = self.frequency_and_randomness.divisor_code();
                self.frequency_and_randomness = FrequencyAndRandomness(value);
                let new_shift = self.frequency_and_randomness.clock_shift();
                let new_code = self.frequency_and_randomness.divisor_code();
                // Combinational tap re-select: a shift change clocks the LFSR if the
                // newly tapped bit sits on its own rising edge; the divider keeps phase.
                if new_shift != old_shift {
                    let tap = (self.divider >> new_shift) & 1 != 0;
                    let tap_prev = (self.divider.wrapping_sub(1) >> new_shift) & 1 != 0;
                    if tap && !tap_prev {
                        self.clock_lfsr();
                    }
                    self.prev_tap = tap;
                }
                // CGB grid-anchors the new cadence: the new code's first terminal
                // lands K new-code periods past the last kanu terminal (K = 2 from
                // code 0, else 1), so preload the prescaler to reach terminal in
                // K·new_code kanu steps. Code 0 is terminal-pinned (immediate).
                if grid_anchor && new_code != old_code && self.enabled.enabled {
                    let k = if old_code == 0 { 2 } else { 1 };
                    if new_code == 0 {
                        // Code 0 is terminal-pinned: off the kanu terminal (jeso
                        // high) the shift divider resumes 4 T ticking on the next
                        // ch4_1mhz rather than waiting for the next kanu terminal.
                        if self.jeso {
                            self.prescaler = 0b111;
                            self.divider_subcounter = 0;
                        }
                    } else {
                        self.prescaler = 0b111u8.saturating_sub(k * new_code) & 0b111;
                    }
                }
            }
            Register::Control => {
                let ctrl = Control(value);

                // gepy = NOR(fexu, bufy_256hz, ff1e_d6_n): length-enable
                // 0→1 rises gepy (one extra length count) iff caru is low.
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
        let was_running = self.enabled.enabled;
        self.enabled.enabled = true;
        self.length.trigger_reload();
        // The divider restarts from 0; its first tap lands at period/2. The
        // cold synchroniser adds a hama-phase-dependent hold so the first tap is
        // at the measured cold-load (sync_delay + period/2): mid-cell / code 0
        // → +4; code 1 hama edge → +8; code ≥ 2 hama edge → +0 (snapped to the
        // 8 T hama grid by the fdis load-settle, code ≥ 1 only).
        self.sync_delay = if self.frequency_and_randomness.divisor_code() == 0 || self.jeso {
            4
        } else if self.frequency_and_randomness.divisor_code() == 1 {
            8
        } else {
            0
        };
        // At double speed the cold-load also depends on the free-running
        // prescaler phase at the trigger — the hold runs to a fixed offset past
        // the next hama terminal (counter 2). Single speed is prescaler-phase-
        // locked (always counter 0), so this term is absent there and on DMG.
        if self.double_speed {
            let dots_to_terminal = (2i16 - self.mhz_prescaler.counter as i16).rem_euclid(4) as u16;
            self.sync_delay += 5 - dots_to_terminal;
        }
        self.divider = 0;
        self.prev_tap = false;
        // ch4_restart reloads the prescaler (huce) and clears gary (guny); the
        // divider stays untouched. The subcounter tracks the 4 T ch4_1mhz grid.
        self.divider_subcounter = 4;
        self.prescaler = !self.frequency_and_randomness.divisor_code() & 0b111;
        self.gary = false;
        // Re-triggering a running channel clocks the first LFSR shift one sample
        // later than a cold trigger: swallow the first tap.
        self.skip_first_clock = was_running;
        self.lfsr = 0x7fff;
        // ch4_restart resets JOPA: any prior armed kyvo is dropped.
        self.envelope.trigger(
            self.volume_and_envelope.initial_volume(),
            self.volume_and_envelope.sweep_pace(),
        );

        // DAC check
        if !self.dac_enabled() {
            self.enabled.enabled = false;
        }
    }

    pub fn tcycle(&mut self, apu_reset_n: bool, t_index: u8, double_speed: bool) {
        self.double_speed = double_speed;
        // ch4_1mhz↑ flips the hama half-phase (jeso); both free-run off the APU
        // clock and are cleared only by apu-off, never by a trigger.
        let mhz_rise = self
            .mhz_prescaler
            .tcycle(apu_reset_n, t_index, double_speed);
        if !apu_reset_n {
            self.jeso = false;
            return;
        }
        if mhz_rise {
            self.jeso = !self.jeso;
        }
        // Cold synchroniser (ch4_restart) holds the frequency timer, then it
        // free-runs. The shift divider is clocked by `noise_counter_clk =
        // ch4_1mhz AND gary`: once per 4 T ch4_1mhz tick the divisor prescaler
        // advances on the 8 T kanu grid (jeso), and gary gates the divider.
        if self.sync_delay > 0 {
            self.sync_delay -= 1;
            return;
        }
        if self.divider_subcounter > 0 {
            self.divider_subcounter -= 1;
        }
        if self.divider_subcounter == 0 {
            self.divider_subcounter = 4;
            // huce (= gary here) reloads ~code; else a kanu step (jeso==0) counts
            // the prescaler up toward terminal 7. gary captures the new terminal.
            if self.prescaler == 0b111 {
                self.prescaler = !self.frequency_and_randomness.divisor_code() & 0b111;
            } else if !self.jeso {
                self.prescaler = (self.prescaler + 1) & 0b111;
            }
            self.gary = self.prescaler == 0b111;
            if self.gary {
                let shift = self.frequency_and_randomness.clock_shift();
                self.divider = self.divider.wrapping_add(1) & 0x3fff;
                let tap = (self.divider >> shift) & 1 != 0;
                let rose = !self.prev_tap && tap;
                self.prev_tap = tap;
                if rose {
                    if self.skip_first_clock {
                        // A re-trigger swallows its first tap (one sample late).
                        self.skip_first_clock = false;
                    } else {
                        self.clock_lfsr();
                    }
                }
            }
        }
    }

    fn clock_lfsr(&mut self) {
        let old_bit0 = self.lfsr & 1;
        let xor_result = old_bit0 ^ ((self.lfsr >> 1) & 1);
        self.lfsr >>= 1;
        self.lfsr |= xor_result << 14;
        // 7-bit width mode
        if self.frequency_and_randomness.short_mode() {
            self.lfsr &= !(1 << 6);
            self.lfsr |= xor_result << 6;
        }
        if self.lfsr & 1 != old_bit0 {
            self.output_dirty = true;
        }
    }

    pub fn tick_length(&mut self) {
        if self.length.tick() {
            self.enabled.enabled = false;
            self.output_dirty = true;
        }
    }

    /// kene↓ edge (fs step 7→0). Advances the envelope counter and arms
    /// `kyvo` on saturation; the volume update is deferred to the next
    /// horu_512hz↑ sample. CH4 has no divider load-settle window (`held`).
    pub fn tick_envelope_counter(&mut self) {
        self.envelope
            .tick_counter(self.volume_and_envelope.sweep_pace(), false);
    }

    /// horu_512hz↑ edge (every fs step transition). Commits an armed `kyvo`
    /// into the volume counter — one 512 Hz tick after the kene↓ that armed it.
    pub fn sample_envelope_jopa(&mut self) {
        if self.envelope.sample_fire(
            self.volume_and_envelope.sweep_pace(),
            self.enabled.enabled,
            self.volume_and_envelope.direction(),
        ) {
            self.output_dirty = true;
        }
    }

    // DAC power = NRx2 upper five bits (GEVY).
    pub fn dac_enabled(&self) -> bool {
        self.volume_and_envelope.0 & 0xf8 != 0
    }

    pub fn digital_sample(&self) -> u8 {
        if !self.enabled.enabled {
            return 0;
        }
        // Output is inverted bit 0 of LFSR
        if self.lfsr & 1 == 0 {
            self.envelope.volume
        } else {
            0
        }
    }
}

struct Control(pub u8);

impl Control {
    const LENGTH: u8 = 0b0100_0000;

    pub fn read(length_enabled: bool) -> u8 {
        if length_enabled {
            0xff
        } else {
            0xff ^ Self::LENGTH
        }
    }

    pub fn trigger(&self) -> bool {
        self.0 & 0b1000_0000 != 0
    }

    pub fn enable_length(&self) -> bool {
        self.0 & Self::LENGTH != 0
    }
}

#[derive(Clone)]
pub struct FrequencyAndRandomness(pub u8);

impl FrequencyAndRandomness {
    pub fn clock_shift(&self) -> u8 {
        self.0 >> 4
    }

    pub fn short_mode(&self) -> bool {
        self.0 & 0b1000 != 0
    }

    pub fn divisor_code(&self) -> u8 {
        self.0 & 0b111
    }
}
