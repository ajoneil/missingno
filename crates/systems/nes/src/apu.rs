//! 2A03 APU basics: both pulses, the triangle, and noise, mixed with the
//! standard nonlinear formula. The DMC is a silent register stub — its
//! sample DMA and bus conflicts are later accuracy work — and the frame
//! counter clocks envelopes and lengths without IRQ wiring.

const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0],
    [1, 0, 0, 1, 1, 1, 1, 1],
];

const NOISE_PERIODS: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

const TRIANGLE_SEQUENCE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15,
];

#[derive(Default)]
struct Envelope {
    start: bool,
    divider: u8,
    decay: u8,
    volume: u8,
    constant: bool,
    looping: bool,
}

impl Envelope {
    fn clock(&mut self) {
        if self.start {
            self.start = false;
            self.decay = 15;
            self.divider = self.volume;
        } else if self.divider == 0 {
            self.divider = self.volume;
            if self.decay > 0 {
                self.decay -= 1;
            } else if self.looping {
                self.decay = 15;
            }
        } else {
            self.divider -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.constant {
            self.volume
        } else {
            self.decay
        }
    }
}

#[derive(Default)]
struct Pulse {
    enabled: bool,
    duty: u8,
    sequence_step: u8,
    period: u16,
    timer: u16,
    length: u8,
    halt_length: bool,
    envelope: Envelope,
    sweep_enabled: bool,
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_divider: u8,
    sweep_reload: bool,
    /// Pulse 1's negate is one deeper than pulse 2's.
    ones_complement: bool,
}

impl Pulse {
    fn sweep_target(&self) -> u16 {
        let change = self.period >> self.sweep_shift;
        if self.sweep_negate {
            self.period
                .wrapping_sub(change)
                .wrapping_sub(self.ones_complement as u16)
        } else {
            self.period.wrapping_add(change)
        }
    }

    fn muted(&self) -> bool {
        self.period < 8 || self.sweep_target() > 0x7FF
    }

    fn clock_sweep(&mut self) {
        if self.sweep_divider == 0 && self.sweep_enabled && self.sweep_shift != 0 && !self.muted() {
            self.period = self.sweep_target() & 0x7FF;
        }
        if self.sweep_divider == 0 || self.sweep_reload {
            self.sweep_divider = self.sweep_period;
            self.sweep_reload = false;
        } else {
            self.sweep_divider -= 1;
        }
    }

    /// The pulse timers run at half the CPU clock.
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.period;
            self.sequence_step = (self.sequence_step + 1) % 8;
        } else {
            self.timer -= 1;
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled
            || self.length == 0
            || self.muted()
            || DUTY_TABLE[self.duty as usize][self.sequence_step as usize] == 0
        {
            0
        } else {
            self.envelope.output()
        }
    }
}

#[derive(Default)]
struct Triangle {
    enabled: bool,
    period: u16,
    timer: u16,
    sequence_step: u8,
    length: u8,
    halt_length: bool,
    linear_counter: u8,
    linear_reload_value: u8,
    linear_reload: bool,
}

impl Triangle {
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.period;
            if self.length > 0 && self.linear_counter > 0 {
                self.sequence_step = (self.sequence_step + 1) % 32;
            }
        } else {
            self.timer -= 1;
        }
    }

    fn clock_linear(&mut self) {
        if self.linear_reload {
            self.linear_counter = self.linear_reload_value;
        } else if self.linear_counter > 0 {
            self.linear_counter -= 1;
        }
        if !self.halt_length {
            self.linear_reload = false;
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled || self.length == 0 || self.linear_counter == 0 {
            0
        } else {
            TRIANGLE_SEQUENCE[self.sequence_step as usize]
        }
    }
}

#[derive(Default)]
struct Noise {
    enabled: bool,
    period: u16,
    timer: u16,
    length: u8,
    halt_length: bool,
    envelope: Envelope,
    mode: bool,
    lfsr: u16,
}

impl Noise {
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.period;
            let tap = if self.mode { 6 } else { 1 };
            let feedback = (self.lfsr ^ (self.lfsr >> tap)) & 1;
            self.lfsr = (self.lfsr >> 1) | (feedback << 14);
        } else {
            self.timer -= 1;
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled || self.length == 0 || self.lfsr & 1 != 0 {
            0
        } else {
            self.envelope.output()
        }
    }
}

pub struct Apu {
    pulse1: Pulse,
    pulse2: Pulse,
    triangle: Triangle,
    noise: Noise,
    frame_step: u8,
    frame_counter: u16,
    five_step: bool,
    half_cycle: bool,
}

impl Default for Apu {
    fn default() -> Self {
        Self::new()
    }
}

impl Apu {
    pub fn new() -> Self {
        Apu {
            pulse1: Pulse {
                ones_complement: true,
                ..Pulse::default()
            },
            pulse2: Pulse::default(),
            triangle: Triangle::default(),
            noise: Noise {
                lfsr: 1,
                ..Noise::default()
            },
            frame_step: 0,
            frame_counter: 0,
            five_step: false,
            half_cycle: false,
        }
    }

    /// One CPU cycle.
    pub fn tick(&mut self) {
        self.triangle.clock_timer();
        self.half_cycle = !self.half_cycle;
        if self.half_cycle {
            self.pulse1.clock_timer();
            self.pulse2.clock_timer();
            self.noise.clock_timer();
        }

        // Quarter frames at ~240 Hz: every 7457 CPU cycles is close
        // enough for the basics; the exact 4/5-step dot sequence is
        // later accuracy work.
        self.frame_counter += 1;
        if self.frame_counter >= 7457 {
            self.frame_counter = 0;
            self.clock_quarter_frame();
            let steps = if self.five_step { 5 } else { 4 };
            self.frame_step = (self.frame_step + 1) % steps;
            if self.frame_step.is_multiple_of(2) {
                self.clock_half_frame();
            }
        }
    }

    fn clock_quarter_frame(&mut self) {
        self.pulse1.envelope.clock();
        self.pulse2.envelope.clock();
        self.noise.envelope.clock();
        self.triangle.clock_linear();
    }

    fn clock_half_frame(&mut self) {
        for pulse in [&mut self.pulse1, &mut self.pulse2] {
            pulse.clock_sweep();
            if !pulse.halt_length && pulse.length > 0 {
                pulse.length -= 1;
            }
        }
        if !self.triangle.halt_length && self.triangle.length > 0 {
            self.triangle.length -= 1;
        }
        if !self.noise.halt_length && self.noise.length > 0 {
            self.noise.length -= 1;
        }
    }

    /// The documented nonlinear mixer, 0.0-1.0.
    pub fn level(&self) -> f32 {
        let pulse = self.pulse1.output() as f32 + self.pulse2.output() as f32;
        let pulse_out = if pulse > 0.0 {
            95.88 / (8128.0 / pulse + 100.0)
        } else {
            0.0
        };
        let tnd = self.triangle.output() as f32 / 8227.0 + self.noise.output() as f32 / 12241.0;
        let tnd_out = if tnd > 0.0 {
            159.79 / (1.0 / tnd + 100.0)
        } else {
            0.0
        };
        pulse_out + tnd_out
    }

    pub fn read(&self, address: u16) -> u8 {
        if address == 0x4015 {
            let mut value = 0;
            if self.pulse1.length > 0 {
                value |= 0x01;
            }
            if self.pulse2.length > 0 {
                value |= 0x02;
            }
            if self.triangle.length > 0 {
                value |= 0x04;
            }
            if self.noise.length > 0 {
                value |= 0x08;
            }
            value
        } else {
            0
        }
    }

    pub fn write(&mut self, address: u16, value: u8) {
        match address {
            0x4000..=0x4007 => {
                let pulse = if address & 0x04 == 0 {
                    &mut self.pulse1
                } else {
                    &mut self.pulse2
                };
                match address & 0x03 {
                    0 => {
                        pulse.duty = value >> 6;
                        pulse.halt_length = value & 0x20 != 0;
                        pulse.envelope.looping = value & 0x20 != 0;
                        pulse.envelope.constant = value & 0x10 != 0;
                        pulse.envelope.volume = value & 0x0F;
                    }
                    1 => {
                        pulse.sweep_enabled = value & 0x80 != 0;
                        pulse.sweep_period = (value >> 4) & 0x07;
                        pulse.sweep_negate = value & 0x08 != 0;
                        pulse.sweep_shift = value & 0x07;
                        pulse.sweep_reload = true;
                    }
                    2 => pulse.period = (pulse.period & 0x700) | value as u16,
                    _ => {
                        pulse.period = (pulse.period & 0xFF) | (((value & 0x07) as u16) << 8);
                        if pulse.enabled {
                            pulse.length = LENGTH_TABLE[(value >> 3) as usize];
                        }
                        pulse.sequence_step = 0;
                        pulse.envelope.start = true;
                    }
                }
            }
            0x4008 => {
                self.triangle.halt_length = value & 0x80 != 0;
                self.triangle.linear_reload_value = value & 0x7F;
            }
            0x400A => {
                self.triangle.period = (self.triangle.period & 0x700) | value as u16;
            }
            0x400B => {
                self.triangle.period =
                    (self.triangle.period & 0xFF) | (((value & 0x07) as u16) << 8);
                if self.triangle.enabled {
                    self.triangle.length = LENGTH_TABLE[(value >> 3) as usize];
                }
                self.triangle.linear_reload = true;
            }
            0x400C => {
                self.noise.halt_length = value & 0x20 != 0;
                self.noise.envelope.looping = value & 0x20 != 0;
                self.noise.envelope.constant = value & 0x10 != 0;
                self.noise.envelope.volume = value & 0x0F;
            }
            0x400E => {
                self.noise.mode = value & 0x80 != 0;
                self.noise.period = NOISE_PERIODS[(value & 0x0F) as usize];
            }
            0x400F => {
                if self.noise.enabled {
                    self.noise.length = LENGTH_TABLE[(value >> 3) as usize];
                }
                self.noise.envelope.start = true;
            }
            0x4015 => {
                self.pulse1.enabled = value & 0x01 != 0;
                self.pulse2.enabled = value & 0x02 != 0;
                self.triangle.enabled = value & 0x04 != 0;
                self.noise.enabled = value & 0x08 != 0;
                for (enabled, length) in [
                    (self.pulse1.enabled, &mut self.pulse1.length),
                    (self.pulse2.enabled, &mut self.pulse2.length),
                    (self.triangle.enabled, &mut self.triangle.length),
                    (self.noise.enabled, &mut self.noise.length),
                ] {
                    if !enabled {
                        *length = 0;
                    }
                }
            }
            0x4017 => {
                self.five_step = value & 0x80 != 0;
                if self.five_step {
                    self.clock_quarter_frame();
                    self.clock_half_frame();
                }
            }
            _ => {}
        }
    }
}
