//! SN76489 PSG, integrated in the VDP package: three tone channels and
//! one noise channel behind a single latch/data write port.

pub struct Psg {
    /// 10-bit tone periods (channel 3's doubles as the noise seed rate).
    pub periods: [u16; 4],
    /// 4-bit attenuations, $F = mute.
    pub volumes: [u8; 4],
    pub noise_control: u8,
    latched_channel: usize,
    latched_volume: bool,

    counters: [u16; 4],
    outputs: [bool; 4],
    noise_lfsr: u16,
    divider: u8,
}

impl Default for Psg {
    fn default() -> Self {
        Self::new()
    }
}

impl Psg {
    pub fn new() -> Self {
        Psg {
            periods: [0; 4],
            volumes: [0x0F; 4],
            noise_control: 0,
            latched_channel: 0,
            latched_volume: false,
            counters: [0; 4],
            outputs: [true; 4],
            noise_lfsr: 0x8000,
            divider: 0,
        }
    }

    pub fn write(&mut self, value: u8) {
        if value & 0x80 != 0 {
            let channel = ((value >> 5) & 0x03) as usize;
            let volume = value & 0x10 != 0;
            self.latched_channel = channel;
            self.latched_volume = volume;
            if volume {
                self.volumes[channel] = value & 0x0F;
            } else if channel == 3 {
                self.noise_control = value & 0x07;
                self.noise_lfsr = 0x8000;
            } else {
                self.periods[channel] = (self.periods[channel] & 0x3F0) | (value & 0x0F) as u16;
            }
        } else if self.latched_volume {
            self.volumes[self.latched_channel] = value & 0x0F;
        } else if self.latched_channel == 3 {
            self.noise_control = value & 0x07;
            self.noise_lfsr = 0x8000;
        } else {
            self.periods[self.latched_channel] =
                (self.periods[self.latched_channel] & 0x00F) | ((value as u16 & 0x3F) << 4);
        }
    }

    /// One CPU T-state; the channel clock is the CPU clock ÷16.
    pub fn tick(&mut self) {
        self.divider += 1;
        if self.divider < 16 {
            return;
        }
        self.divider = 0;

        for channel in 0..3 {
            if self.counters[channel] == 0 {
                self.counters[channel] = self.periods[channel];
                self.outputs[channel] = !self.outputs[channel];
            } else {
                self.counters[channel] -= 1;
            }
        }

        if self.counters[3] == 0 {
            self.counters[3] = match self.noise_control & 0x03 {
                0 => 0x10,
                1 => 0x20,
                2 => 0x40,
                _ => self.periods[2],
            };
            self.outputs[3] = !self.outputs[3];
            // The LFSR shifts on the flip-flop's rising edge.
            if self.outputs[3] {
                let white = self.noise_control & 0x04 != 0;
                let input = if white {
                    (self.noise_lfsr ^ (self.noise_lfsr >> 3)) & 1
                } else {
                    self.noise_lfsr & 1
                };
                self.noise_lfsr = (self.noise_lfsr >> 1) | (input << 15);
            }
        }
    }

    /// Summed output, 0.0-1.0.
    pub fn level(&self) -> f32 {
        let mut sum = 0.0;
        for channel in 0..3 {
            if self.outputs[channel] {
                sum += amplitude(self.volumes[channel]);
            }
        }
        if self.noise_lfsr & 1 != 0 {
            sum += amplitude(self.volumes[3]);
        }
        sum / 4.0
    }
}

/// 2 dB per attenuation step, $F fully mute.
fn amplitude(attenuation: u8) -> f32 {
    if attenuation == 0x0F {
        0.0
    } else {
        10.0f32.powf(-0.1 * attenuation as f32)
    }
}
