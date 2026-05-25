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

    /// Divider down-counter; reloads `(2048 - period)` on each
    /// `ch3_frst ↑`, held at the loaded value while `ch3_restart` is
    /// high (load mode via `hera = NOR(ch3_frst, ch3_restart)`).
    pub frequency_timer: u16,
    pub wave_position: u8,
    pub length_counter: u16,

    /// `cery` /2 prescaler — toggles on every master-clock rise; held
    /// at 0 while `apu_reset = 1`.
    pub ch3_2mhz: bool,
    /// `huno` DFF clock-to-q ripple: one master-clock edge between
    /// divider wrap and `ch3_frst ↑`.
    pub pending_overflow: bool,

    /// Trigger sync stage 1 — captures NR34 d7 at `apu_wr ↑`.
    pub gavu: bool,
    /// Trigger sync stage 2 — captures `gavu` at the M-cycle boundary
    /// (`apu_phi ↑` = T=0 rise of M+1).
    pub foba: bool,
    /// `gara` — captures the gofy_n-armed trigger pulse on `fabo ↑`
    /// (= `ch3_2mhz ↓`). Held high for one `ch3_2mhz` cycle.
    pub ch3_restart: bool,
    /// Self-clear driver — samples `ch3_restart` on `fabo ↑` and on
    /// the following `fabo ↑` async-resets `gara` via `fury`.
    pub gyta: bool,

    /// Overflow capture pulse from `huno`; held one `ch3_2mhz` cycle
    /// and cleared by `hupa = AND(huno, ch3_2mhz)`.
    pub ch3_frst: bool,
    /// First wave-data-latch sync stage — captures `ch3_frst` on
    /// `apu_4mhz ↑` (= our fall edge).
    pub busa: bool,
    /// Second sync stage — captures `busa` on `cozy ↑` (= our rise).
    pub bano: bool,
    /// Third sync stage = `wave_data_latch` — captures `bano` on
    /// `apu_4mhz ↑`. CPU reads of FF30..FF3F return the wave-RAM byte
    /// while this is high; outside the window the bus floats to 0xFF.
    pub azus: bool,
    /// `azus` re-captured on `apu_4mhz ↓` to extend the SRAM-read-
    /// active window. `(azus | azet) = 1` drives
    /// `wave_ram_bl_precharge = 0` and is the corruption gate on
    /// `ch3_restart ↑`.
    pub azet: bool,
    /// NAND-latch gating `juty` (the divider toggle clock). Set by
    /// DAC-off or `apu_reset`; cleared by the gyta-derived `s_n`
    /// pulse on a trigger. While set, no divider overflows fire.
    pub ch3_fdis: bool,
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
            ch3_2mhz: false,
            pending_overflow: false,
            gavu: false,
            foba: false,
            ch3_restart: false,
            gyta: false,
            ch3_frst: false,
            busa: false,
            bano: false,
            azus: false,
            azet: false,
            ch3_fdis: true,
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
            ch3_2mhz: false,
            pending_overflow: false,
            gavu: false,
            foba: false,
            ch3_restart: false,
            gyta: false,
            ch3_frst: false,
            busa: false,
            bano: false,
            azus: false,
            azet: false,
            ch3_fdis: true,
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
                    // DAC-off raises `ch3_amp_en_n` which sets the
                    // `ch3_fdis` NAND-latch — divider clock gated.
                    self.ch3_fdis = true;
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
        // `gavu` is a level-sensitive drlatch; capture from commit_write
        // is equivalent to hardware's apu_wr↑ capture.
        self.gavu = true;

        self.enabled.enabled = true;
        if self.length_counter == 0 {
            self.length_counter = 256;
        }

        if !self.dac_enabled {
            self.enabled.enabled = false;
        }
    }

    pub fn tcycle(&mut self, t_index: u8, apu_reset_n: bool) {
        // `cery` async-reset: held at 0 while `apu_reset = 1`. The
        // downstream divider / synchroniser chain is held inert via
        // `ch3_fdis = 1` and so doesn't need separate gating.
        if !apu_reset_n {
            self.ch3_2mhz = false;
            return;
        }

        let ch3_2mhz_prev = self.ch3_2mhz;
        self.ch3_2mhz = !self.ch3_2mhz;
        let ch3_2mhz_rising = !ch3_2mhz_prev && self.ch3_2mhz;
        let fabo_rising = ch3_2mhz_prev && !self.ch3_2mhz; // ch3_2mhz↓

        // huno clock-to-q ripple: promote the pending overflow into
        // `ch3_frst` on the rise AFTER the divider wrap.
        if self.pending_overflow {
            self.ch3_frst = true;
            self.pending_overflow = false;
        }

        // wave_data_latch sync chain: BANO captures BUSA on cozy↑
        // (rise), AZET captures AZUS on apu_4mhz↓ (rise), so the
        // `(azus | azet)` SRAM-read-active window spans 1.5 T-cycles.
        self.bano = self.busa;
        self.azet = self.azus;

        // foba captures gavu on apu_phi↑ (= M-cycle boundary, T=0).
        if t_index == 0 {
            self.foba = self.gavu;
        }

        if fabo_rising {
            // gara/gyta sample on fabo↑. gyta captures the prior
            // ch3_restart; when high it async-resets gara via fury.
            let new_gyta = self.ch3_restart;
            if new_gyta {
                self.ch3_restart = false;
                self.foba = false;
                self.gavu = false;
                self.gyta = true;
                self.on_ch3_restart_fall();
            } else if self.foba {
                self.gyta = false;
                self.ch3_restart = true;
                self.on_ch3_restart_rise();
            } else {
                self.gyta = false;
            }
        }

        // hupa = AND(huno, ch3_2mhz): clears ch3_frst on the next
        // ch3_2mhz↑. The wave-position counter advances on this edge
        // (= dero↑), one ch3_2mhz cycle after the overflow.
        if ch3_2mhz_rising && self.ch3_frst {
            self.ch3_frst = false;
            self.wave_position = (self.wave_position + 1) % 32;
        }

        // Divider clocks on ch3_2mhz↑ when hera is high (= ch3_restart
        // and ch3_frst both low — out of load mode) and juty is active
        // (= ch3_fdis = 0, channel enabled).
        if ch3_2mhz_rising
            && !self.ch3_restart
            && !self.ch3_frst
            && !self.pending_overflow
            && !self.ch3_fdis
            && self.frequency_timer > 0
        {
            self.frequency_timer -= 1;
            if self.frequency_timer == 0 {
                self.frequency_timer = 2048 - self.period.0 as u16;
                self.pending_overflow = true;
            }
        }
    }

    /// `ch3_restart ↑` effects: wave-RAM corruption if the SRAM
    /// bit-lines are still driven (= `(azus | azet) = 1`), then
    /// wave-position async-reset and divider load.
    fn on_ch3_restart_rise(&mut self) {
        // Retrigger-during-active-read shorts SRAM wordlines, copying
        // part of CH3's currently-read row into ram[0..3]. With
        // byte_pos < 4 (source row == destination row 0) only one
        // cell shorts, so just ram[0] gets ram[byte_pos]. With
        // byte_pos >= 4, four column wordlines are enabled across
        // both rows and the full 4-byte row copies through.
        if self.azus || self.azet {
            let byte_pos = (self.wave_position as usize) >> 1;
            if byte_pos < 4 {
                self.ram[0] = self.ram[byte_pos];
            } else {
                let src = (byte_pos >> 2) * 4;
                for i in 0..4 {
                    self.ram[i] = self.ram[src + i];
                }
            }
        }

        // etan ↓ async-resets the wave-position counter; hera ↓ opens
        // the divider load window.
        self.wave_position = 0;
        self.frequency_timer = 2048 - self.period.0 as u16;
        self.ch3_frst = false;
        self.pending_overflow = false;
    }

    /// `ch3_restart ↓` (gyta-driven self-clear): divider exits load
    /// mode on the next cery↑; `ch3_fdis` is cleared on the same edge.
    fn on_ch3_restart_fall(&mut self) {
        self.ch3_fdis = false;
    }

    /// Half-T-cycle synchroniser step on master-clock fall (=
    /// apu_4mhz ↑ at mid-T-cycle). Captures the two DFFs in the
    /// `busa → bano → azus` chain that clock on apu_4mhz ↑.
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
    /// Active-channel reads return the byte at the current wave
    /// position only while `wave_data_latch` (= `azus`) is high —
    /// outside that ~1-T-cycle pulse the bus floats to 0xFF.
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

    /// Active-channel writes target `ram[wave_position[4:1]]` (= the
    /// byte CH3 is reading) but only commit when the SRAM wordline
    /// driver is out of precharge, which tracks `azus`. The `(azus |
    /// azet)` check at the T=3 fall commit edge covers both half-T
    /// edges of T=3 of the `wave_ram_wr` pulse — sufficient for every
    /// alignment the test ROMs exercise.
    pub fn write_wave_ram(&mut self, offset: u8, value: u8) {
        let ch3 = &mut self.channels.ch3;
        if !ch3.enabled.enabled {
            ch3.ram[offset as usize] = value;
            return;
        }
        if ch3.azus || ch3.azet {
            let byte_idx = ch3.wave_position as usize / 2;
            ch3.ram[byte_idx] = value;
        }
    }
}
