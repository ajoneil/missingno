use super::{
    Enabled,
    registers::{EnvelopeDirection, VolumeAndEnvelope},
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
    pub length_enabled: bool,
    pub frequency_and_randomness: FrequencyAndRandomness,

    pub frequency_timer: u16,
    pub lfsr: u16,
    pub current_volume: u8,
    pub envelope_timer: u8,
    /// Stop latch (CH4 mirror of CH1/CH2's JEME): a fire that samples a
    /// saturated volume counter latches it until the next trigger.
    pub envelope_stopped: bool,
    /// Envelope-fire arm (CH4 mirror of CH1/CH2's `kyvo`/JOPA): set at kene↓
    /// on counter saturation; the volume commit is deferred to the next
    /// horu_512hz↑ sample.
    pub kyvo: bool,
    pub length_counter: u16,
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
            length_enabled: false,
            frequency_and_randomness: FrequencyAndRandomness(0),

            frequency_timer: 0,
            lfsr: 0x7fff,
            current_volume: 0,
            envelope_timer: 0,
            envelope_stopped: false,
            kyvo: false,
            length_counter: 0,
        }
    }
}

impl NoiseChannel {
    pub fn reset(&mut self) {
        let length_counter = self.length_counter; // DMG: NR41 length timer preserved on power-off
        self.enabled = Enabled::disabled();
        self.volume_and_envelope = VolumeAndEnvelope(0);
        self.length_enabled = false;
        self.frequency_and_randomness = FrequencyAndRandomness(0);

        self.frequency_timer = 0;
        self.lfsr = 0x7fff;
        self.current_volume = 0;
        self.envelope_timer = 0;
        self.envelope_stopped = false;
        self.kyvo = false;
        self.length_counter = length_counter;
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::LengthTimer => 0xff,
            Register::VolumeAndEnvelope => self.volume_and_envelope.0,
            Register::FrequencyAndRandomness => self.frequency_and_randomness.0,
            Register::Control => Control::read(self.length_enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8, caru_low: bool) {
        match register {
            Register::LengthTimer => {
                self.length_counter = 64 - (value & 0x3f) as u16;
            }
            Register::VolumeAndEnvelope => {
                // Write-strobe transient (CH4 mirror of CH1/CH2): one +1
                // volume clock iff the old pace was 0, free 4-bit wrap.
                if self.volume_and_envelope.sweep_pace() == 0 && !self.envelope_stopped {
                    self.current_volume = (self.current_volume + 1) & 0xf;
                }
                self.volume_and_envelope = VolumeAndEnvelope(value);
                // Disabling the DAC immediately disables the channel
                if value & 0xf8 == 0 {
                    self.enabled.enabled = false;
                }
            }
            Register::FrequencyAndRandomness => {
                self.frequency_and_randomness = FrequencyAndRandomness(value)
            }
            Register::Control => {
                let ctrl = Control(value);

                // gepy = NOR(fexu, bufy_256hz, ff1e_d6_n): length-enable
                // 0→1 rises gepy (one extra length count) iff caru is low.
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
        self.frequency_timer = self.frequency_and_randomness.timer_period();
        self.lfsr = 0x7fff;
        self.current_volume = self.volume_and_envelope.initial_volume();
        self.envelope_timer = self.volume_and_envelope.sweep_pace();
        self.envelope_stopped = false;
        // ch4_restart resets JOPA: any prior armed kyvo is dropped.
        self.kyvo = false;

        // DAC check
        if self.volume_and_envelope.0 & 0xf8 == 0 {
            self.enabled.enabled = false;
        }
    }

    pub fn tcycle(&mut self, apu_reset_n: bool) {
        if !apu_reset_n {
            return;
        }
        if self.frequency_timer > 0 {
            self.frequency_timer -= 1;
        }
        if self.frequency_timer == 0 {
            self.frequency_timer = self.frequency_and_randomness.timer_period();

            // Clock LFSR
            let xor_result = (self.lfsr & 1) ^ ((self.lfsr >> 1) & 1);
            self.lfsr >>= 1;
            self.lfsr |= xor_result << 14;

            // 7-bit width mode
            if self.frequency_and_randomness.short_mode() {
                self.lfsr &= !(1 << 6);
                self.lfsr |= xor_result << 6;
            }
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

    /// kene↓ edge (fs step 7→0). Advances the envelope counter and arms
    /// `kyvo` on saturation; the volume update is deferred to the next
    /// horu_512hz↑ sample.
    pub fn tick_envelope_counter(&mut self) {
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

    /// horu_512hz↑ edge (every fs step transition). Commits an armed `kyvo`
    /// into the volume counter — one 512 Hz tick after the kene↓ that armed it.
    pub fn sample_envelope_jopa(&mut self) {
        if !self.kyvo {
            return;
        }
        self.kyvo = false;
        let pace = self.volume_and_envelope.sweep_pace();
        if pace == 0 || !self.enabled.enabled || self.envelope_stopped {
            return;
        }
        // A fire that samples a saturated counter latches the stop
        // instead of stepping — no arithmetic clamp.
        match self.volume_and_envelope.direction() {
            EnvelopeDirection::Increase => {
                if self.current_volume == 15 {
                    self.envelope_stopped = true;
                } else {
                    self.current_volume += 1;
                }
            }
            EnvelopeDirection::Decrease => {
                if self.current_volume == 0 {
                    self.envelope_stopped = true;
                } else {
                    self.current_volume -= 1;
                }
            }
        }
    }

    pub fn digital_sample(&self) -> u8 {
        if !self.enabled.enabled {
            return 0;
        }
        // Output is inverted bit 0 of LFSR
        if self.lfsr & 1 == 0 {
            self.current_volume
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

    fn timer_period(&self) -> u16 {
        let divisor = match self.divisor_code() {
            0 => 8,
            n => (n as u16) * 16,
        };
        divisor << self.clock_shift()
    }
}
