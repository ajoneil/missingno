use super::super::Audio;
use super::{
    Enabled,
    registers::{PeriodHighAndControl, Signed11},
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
    /// `ch3_frst` overflow capture pulse — high from `ch3_frst↑` to
    /// `ch3_frst↓` (one `ch3_2mhz` cycle = 2 T-cycles). Drives the
    /// BUSA/BANO/AZUS synchroniser. Spec §14.8.6.
    pub ch3_frst: bool,
    /// Countdown to `ch3_frst↓` after `ch3_frst↑` — 2 T-cycles of held
    /// high (= one `ch3_2mhz` cycle). 0 = idle.
    pub ch3_frst_remaining: u8,
    /// `busa` DFF (§14.8.4) — captures `ch3_frst` on apu_4mhz↑ (our
    /// fall edge). First synchroniser stage.
    pub busa: bool,
    /// `bano` DFF — captures `busa` on cozy↑ (our rise edge). Second
    /// stage.
    pub bano: bool,
    /// `azus` DFF — captures `bano` on apu_4mhz↑ (our fall edge).
    /// Buffered to `wave_data_latch`. Third stage; while true the
    /// wave-RAM block drives the bus.
    pub azus: bool,
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
            ch3_frst: false,
            ch3_frst_remaining: 0,
            busa: false,
            bano: false,
            azus: false,
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
            ch3_frst: false,
            ch3_frst_remaining: 0,
            busa: false,
            bano: false,
            azus: false,
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
        // DMG wave-RAM corruption (§14.8.5): ch3_restart↑ async-resets
        // wave_position to 0, but if the SRAM bit-lines still hold the
        // prior byte's value (wave_data_latch high), the wordline at
        // address 0 captures the byte at the pre-reset position. Pan
        // Docs Audio_details.html: byte_pos < 4 copies one byte; else
        // a 4-byte aligned block.
        if self.enabled.enabled && self.azus {
            let byte_pos = (self.wave_position as usize) / 2;
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
        // ch3_restart synchroniser delay per §14.8.3: ~6 T-cycles from
        // apu_wr↑ to ch3_restart↑, plus 2 T-cycles ch3_restart held →
        // the divider starts counting ~8 T-cycles after the write,
        // but apu_wr↑ leads our trigger() call site (T=3 fall =
        // apu_wr↓) by 1.5 T-cycles. Net: +6 T-cycles before our
        // frequency_timer begins natural countdown.
        self.frequency_timer = (2048 - self.period.0) * 2 + 6;
        self.wave_position = 0;
        // ch3_frst / wave_data_latch chain is async-reset by ch3_restart
        // — clear all synchroniser state.
        self.ch3_frst = false;
        self.ch3_frst_remaining = 0;
        self.busa = false;
        self.bano = false;
        self.azus = false;

        if !self.dac_enabled {
            self.enabled.enabled = false;
        }
    }

    pub fn tcycle(&mut self, _t_index: u8) {
        // BANO captures BUSA at cozy↑ (= apu_4mhz↓ = master-clock rise).
        // Second of three apu_4mhz edges in the wave_data_latch chain.
        self.bano = self.busa;

        // ch3_frst self-clears one ch3_2mhz cycle (2 T-cycles) after
        // ch3_frst↑ via the hupa = AND(huno, ch3_2mhz) self-clear.
        if self.ch3_frst_remaining > 0 {
            self.ch3_frst_remaining -= 1;
            if self.ch3_frst_remaining == 0 {
                self.ch3_frst = false;
            }
        }

        if self.frequency_timer > 0 {
            self.frequency_timer -= 1;
        }
        if self.frequency_timer == 0 {
            self.frequency_timer = (2048 - self.period.0) * 2;
            self.wave_position = (self.wave_position + 1) % 32;
            // ch3_frst↑ — held until next ch3_2mhz↑ self-clear (§14.8.6).
            // Phase-dependent (~1-2 T-cycles per FST trace); 1 rise-tick
            // gives wave_data_latch the spec-anchored ~1 T-cycle width.
            self.ch3_frst = true;
            self.ch3_frst_remaining = 1;
        }
    }

    /// Half-T-cycle synchroniser step on master-clock fall edge
    /// (= apu_4mhz↑ at mid-T-cycle). BUSA captures `ch3_frst`, AZUS
    /// captures BANO. Together with BANO's rise-edge capture in
    /// `tcycle()`, this implements the 3-stage `busa → bano → azus`
    /// chain that gates `wave_data_latch` per §14.8.4.
    pub fn fall_sync(&mut self) {
        self.azus = self.bano;
        self.busa = self.ch3_frst;
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
    /// DMG wave-RAM bus alignment per §14.8.4. The wave-RAM SRAM array
    /// drives the bus only while `wave_data_latch` (= AZUS) is high.
    /// The strobe rises ~1.5 T-cycles after `ch3_frst↑` (the BUSA →
    /// BANO → AZUS synchroniser) and is ~1 T-cycle wide. Accesses
    /// outside that window see a floating bus → 0xFF.
    /// `bus_value_at_latch` re-evaluates so accesses landing late in
    /// the M-cycle still see windows that opened after the
    /// drive-enable snapshot was taken.
    pub fn read_wave_ram(&self, offset: u8) -> u8 {
        let ch3 = &self.channels.ch3;
        if !ch3.enabled.enabled {
            return ch3.ram[offset as usize];
        }
        if ch3.azus {
            ch3.ram[ch3.wave_position as usize / 2]
        } else {
            0xFF
        }
    }

    pub fn write_wave_ram(&mut self, offset: u8, value: u8) {
        let ch3 = &mut self.channels.ch3;
        if !ch3.enabled.enabled {
            ch3.ram[offset as usize] = value;
            return;
        }
        if ch3.azus {
            let byte_idx = ch3.wave_position as usize / 2;
            ch3.ram[byte_idx] = value;
        }
    }
}
