use crate::game_boy::audio::{
    Audio,
    channels::{
        Enabled,
        registers::{PeriodHighAndControl, Signed11},
    },
};

#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    Volume,
    DacEnabled,
    Length,
    PeriodLow,
    PeriodHighAndControl,
}

#[derive(Clone)]
pub struct WaveChannel {
    pub enabled: Enabled,
    pub dac_enabled: bool,
    pub volume: Volume,
    pub length_enabled: bool,
    pub period: Signed11,
    pub ram: [u8; 16],

    pub frequency_timer: u16,
    pub wave_position: u8,
    pub length_counter: u16,
    /// Which T-cycle (0-3) within the last M-cycle batch CH3 read a sample on.
    /// Set to 0xFF if no read happened in the last M-cycle. Used for DMG wave
    /// RAM access timing: CPU can only access wave RAM on the same T-cycle.
    pub sample_read_tcycle: u8,
}

impl Default for WaveChannel {
    fn default() -> Self {
        Self {
            enabled: Enabled {
                enabled: false,
                output_left: true,
                output_right: false,
            },
            dac_enabled: false,
            volume: Volume(0x9f),
            length_enabled: false,
            period: (-1).into(),
            ram: [0; 16],

            frequency_timer: 0,
            wave_position: 0,
            length_counter: 0,
            sample_read_tcycle: 0xFF,
        }
    }
}

impl WaveChannel {
    pub fn reset(&mut self) {
        let ram = self.ram; // Wave RAM is preserved across APU power off
        let length_counter = self.length_counter; // DMG: length timers preserved on power-off
        *self = Self {
            enabled: Enabled::disabled(),
            dac_enabled: false,
            volume: Volume(0),
            length_enabled: false,
            period: 0.into(),
            ram,

            frequency_timer: 0,
            wave_position: 0,
            length_counter,
            sample_read_tcycle: 0xFF,
        };
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Volume => self.volume.0 | 0x9F,
            Register::Length => 0xff,
            Register::DacEnabled => {
                if self.dac_enabled {
                    0xff
                } else {
                    0x7f
                }
            }
            Register::PeriodLow => 0xff,
            Register::PeriodHighAndControl => PeriodHighAndControl::read(self.length_enabled),
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8, frame_sequencer_step: u8) {
        match register {
            Register::Volume => self.volume = Volume(value),
            Register::Length => {
                self.length_counter = 256 - value as u16;
            }
            Register::DacEnabled => {
                self.dac_enabled = value & 0b1000_0000 != 0;
                if !self.dac_enabled {
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
                    if !next_step_clocks_length && self.length_enabled && self.length_counter == 256
                    {
                        self.length_counter = 255;
                    }
                }
            }
        }
    }

    pub fn trigger(&mut self) {
        // DMG: triggering while CH3 is active corrupts wave RAM, but only
        // when the re-trigger coincides with CH3's frequency timer having
        // just expired. In SameBoy this is `sample_countdown == 0`. In our
        // T-cycle model, the timer reloads immediately on expiry. A read on
        // T3 (last T-cycle of the batch) means the timer reloaded and no
        // further decrements happened, so frequency_timer == reload value.
        let reload = (2048 - self.period.0) * 2;
        if self.enabled.enabled && self.frequency_timer == reload {
            let byte_pos = ((self.wave_position + 1) / 2) as usize & 0xF;
            if byte_pos < 4 {
                self.ram[0] = self.ram[byte_pos];
            } else {
                let aligned = byte_pos & !3;
                self.ram[0] = self.ram[aligned];
                self.ram[1] = self.ram[aligned + 1];
                self.ram[2] = self.ram[aligned + 2];
                self.ram[3] = self.ram[aligned + 3];
            }
        }

        self.enabled.enabled = true;
        if self.length_counter == 0 {
            self.length_counter = 256;
        }
        // Extra delay on trigger before first sample read. Real hardware
        // delays +6 T-cycles (SameBoy: +3 APU ticks). Our M-cycle batch model
        // runs 4 T-cycles of audio in the same M-cycle as the trigger write,
        // so the effective offset is +8 to compensate.
        self.frequency_timer = (2048 - self.period.0) * 2 + 8;
        self.wave_position = 0;

        if !self.dac_enabled {
            self.enabled.enabled = false;
        }
    }

    pub fn tcycle(&mut self, t_index: u8) {
        if t_index == 0 {
            self.sample_read_tcycle = 0xFF; // reset at start of M-cycle
        }
        if self.frequency_timer > 0 {
            self.frequency_timer -= 1;
        }
        if self.frequency_timer == 0 {
            self.frequency_timer = (2048 - self.period.0) * 2;
            self.wave_position = (self.wave_position + 1) % 32;
            self.sample_read_tcycle = t_index;
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

    pub fn sample(&self) -> f32 {
        if !self.enabled.enabled {
            return 0.0;
        }
        let byte = self.ram[self.wave_position as usize / 2];
        let nibble = if self.wave_position.is_multiple_of(2) {
            byte >> 4
        } else {
            byte & 0x0f
        };
        let volume_shift = self.volume.shift();
        if volume_shift == 0 {
            return 0.0;
        }
        (nibble >> (volume_shift - 1)) as f32 / 15.0
    }
}

#[derive(Clone)]
pub struct Volume(pub u8);
impl Volume {
    pub fn volume(&self) -> f32 {
        ((self.0 >> 5) & 0b11) as f32 / 4.0
    }

    fn shift(&self) -> u8 {
        match (self.0 >> 5) & 0b11 {
            0 => 0, // mute
            1 => 1, // 100%
            2 => 2, // 50%
            3 => 3, // 25% (shift right by 2, but we return the code for the caller)
            _ => unreachable!(),
        }
    }
}

impl Audio {
    /// Simulate T0 and T1 of the current M-cycle to check if CH3 reads a
    /// sample on T1. On DMG, wave RAM is only accessible on the exact T-cycle
    /// that CH3 reads. The CPU read/write happens at T1. Returns the byte
    /// index into wave RAM if access succeeds, or None if it would return $FF.
    fn ch3_wave_ram_access_byte(&self) -> Option<usize> {
        let ch3 = &self.channels.ch3;
        if !ch3.enabled.enabled {
            return None;
        }
        let reload = (2048 - ch3.period.0) * 2;
        let mut timer = ch3.frequency_timer;
        let mut position = ch3.wave_position;
        // T0: decrement
        if timer > 0 {
            timer -= 1;
        }
        if timer == 0 {
            timer = reload;
            position = (position + 1) % 32;
        }
        // T1: decrement — if it hits 0, CH3 reads on T1
        if timer > 0 {
            timer -= 1;
        }
        if timer == 0 {
            // CH3 advances position and reads the byte at the new position.
            position = (position + 1) % 32;
            Some(position as usize / 2)
        } else {
            None
        }
    }

    pub fn read_wave_ram(&self, offset: u8) -> u8 {
        let ch3 = &self.channels.ch3;
        if ch3.enabled.enabled {
            // DMG: wave RAM can only be read on the same T-cycle CH3 reads.
            if let Some(byte_idx) = self.ch3_wave_ram_access_byte() {
                ch3.ram[byte_idx]
            } else {
                0xFF
            }
        } else {
            ch3.ram[offset as usize]
        }
    }

    pub fn write_wave_ram(&mut self, offset: u8, value: u8) {
        if self.channels.ch3.enabled.enabled {
            // DMG: wave RAM can only be written on the same T-cycle CH3 reads.
            if let Some(byte_idx) = self.ch3_wave_ram_access_byte() {
                self.channels.ch3.ram[byte_idx] = value;
            }
        } else {
            self.channels.ch3.ram[offset as usize] = value;
        }
    }
}
