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

    /// Divider counter — counts `ch3_2mhz↑` ticks remaining until the
    /// next natural overflow. Reloaded to `(2048 - period)` on each
    /// `ch3_frst↑`. Held at the loaded value while `ch3_restart` is
    /// high (load mode via `hera = NOR(ch3_frst, ch3_restart)`).
    pub frequency_timer: u16,
    pub wave_position: u8,
    pub length_counter: u16,

    /// `cery` DFF (§14.8.1) — `ch3_2mhz` prescaler /2 stage. Toggles
    /// on every `cybo↑` (= master-clock rise, = our `rise()` edge).
    /// Free-running, reset only by `apu_reset`.
    pub ch3_2mhz: bool,
    /// Set on `ch3_restart↓` to skip the first `ch3_2mhz↑` after the
    /// load window closes — the divider DFFs settle out of load mode
    /// on the first edge but only begin counting on the second.
    /// Accounts for the "2 T-cycles divider count" entry in §14.8.3's
    /// trigger-delay decomposition.
    pub divider_load_settle: bool,

    /// Captures NR34 d7 at `apu_wr↑` (in our model: at trigger() time,
    /// the commit_write edge). Held until consumed by `foba` at the
    /// next M-cycle boundary. Spec §14.8.3 stage 1.
    pub gavu: bool,
    /// `foba` DFF — captures `gavu` at the next M-cycle boundary
    /// (`apu_phi↑`, = T=0 rise of M+1). Spec §14.8.3 stage 2.
    pub foba: bool,
    /// `gara` DFF (= `ch3_restart`) — captures the gofy_n-armed
    /// trigger pulse on `fabo↑` (= `ch3_2mhz↓`). Held high for one
    /// `ch3_2mhz` cycle (= 2 T-cycles). Spec §14.8.3 stages 4-5.
    pub ch3_restart: bool,
    /// `gyta` DFF — samples `ch3_restart` on `fabo↑` to drive the
    /// self-clear async-reset of `gara` on the following `fabo↑`.
    pub gyta: bool,

    /// `ch3_frst` overflow capture pulse — held high for one
    /// `ch3_2mhz` cycle after the divider overflow. Drives the
    /// BUSA/BANO/AZUS wave-RAM bus synchroniser. Spec §14.8.6.
    pub ch3_frst: bool,
    /// `busa` DFF (§14.8.4) — captures `ch3_frst` on `apu_4mhz↑`
    /// (= our fall edge). First synchroniser stage.
    pub busa: bool,
    /// `bano` DFF — captures `busa` on `cozy↑` (= our rise edge).
    /// Second stage.
    pub bano: bool,
    /// `azus` DFF — captures `bano` on `apu_4mhz↑` (= our fall edge).
    /// Buffered to `wave_data_latch`. Third stage; while true the
    /// wave-RAM block drives the bus.
    pub azus: bool,
    /// `azet` DFF — captures `azus` on `apu_4mhz↓` (= T-cycle start
    /// = our rise edge). Together with `azus`, `(azus | azet)` defines
    /// the wave-RAM SRAM-read-active window driving
    /// `wave_ram_bl_precharge = NOT(NOR(azus, azet))`. The corruption
    /// gate on `ch3_restart ↑` fires within this 1.5-T-cycle window
    /// per §14.8.5 (resolved 2026-05-24).
    pub azet: bool,
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
            divider_load_settle: false,
            gavu: false,
            foba: false,
            ch3_restart: false,
            gyta: false,
            ch3_frst: false,
            busa: false,
            bano: false,
            azus: false,
            azet: false,
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
            divider_load_settle: false,
            gavu: false,
            foba: false,
            ch3_restart: false,
            gyta: false,
            ch3_frst: false,
            busa: false,
            bano: false,
            azus: false,
            azet: false,
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
        // NR34 d7 latches into `gavu` at apu_wr↑ (§14.8.3 stage 1).
        // In our model the trigger() call is at commit_write (= apu_wr↓
        // edge); equivalent for capture since gavu is a level-sensitive
        // drlatch that holds the value past apu_wr↓.
        self.gavu = true;

        self.enabled.enabled = true;
        if self.length_counter == 0 {
            self.length_counter = 256;
        }

        if !self.dac_enabled {
            self.enabled.enabled = false;
        }
    }

    pub fn tcycle(&mut self, t_index: u8) {
        // `cery` toggles on every `cybo↑` = master-clock rise. Drives
        // `ch3_2mhz`; under quickboot phase the rise lands at T=0 / T=2
        // atal↑ and the fall at T=1 / T=3 (§14.8.7).
        let ch3_2mhz_prev = self.ch3_2mhz;
        self.ch3_2mhz = !self.ch3_2mhz;
        let ch3_2mhz_rising = !ch3_2mhz_prev && self.ch3_2mhz;
        let fabo_rising = ch3_2mhz_prev && !self.ch3_2mhz; // ch3_2mhz↓

        // BANO captures BUSA at `cozy↑` = our rise edge (§14.8.4).
        // AZET captures AZUS on the same edge (apu_4mhz↓ = T-cycle
        // start), holding the prior T-cycle's wave_data_latch value
        // so the SRAM-read-active window `(azus | azet)` spans 1.5
        // T-cycles (§14.8.5, §14.8.7).
        self.bano = self.busa;
        self.azet = self.azus;

        // `foba` (§14.8.3 stage 2) — DFF clocked by `apu_phi`, which
        // rises at the M-cycle boundary. T=0 rise of every M-cycle.
        if t_index == 0 {
            self.foba = self.gavu;
        }

        // `gara` / `gyta` synchroniser sample on `fabo↑` (§14.8.3
        // stages 4-5). gyta captures the prior ch3_restart; if it
        // becomes 1 then the async-reset path forces ch3_restart to 0.
        if fabo_rising {
            let new_gyta = self.ch3_restart;
            if new_gyta {
                // Self-clear via gyta→fury→gara.r_n. Also clears the
                // gofy_n path so the next fabo↑ samples 0.
                self.ch3_restart = false;
                self.foba = false;
                self.gavu = false;
                self.gyta = true;
                self.on_ch3_restart_fall();
            } else if self.foba {
                // First fabo↑ with foba armed → ch3_restart↑.
                self.gyta = false;
                self.ch3_restart = true;
                self.on_ch3_restart_rise();
            } else {
                self.gyta = false;
            }
        }

        // ch3_frst is held high for one `ch3_2mhz` cycle — clears on
        // the next `ch3_2mhz↑` via hupa = AND(huno, ch3_2mhz) (§14.8.6).
        if ch3_2mhz_rising && self.ch3_frst {
            self.ch3_frst = false;
        }

        // Divider clocks on `ch3_2mhz↑` while not in load mode (hera
        // is high = NOR(ch3_frst, ch3_restart) = 1 means both low).
        if ch3_2mhz_rising && !self.ch3_restart && !self.ch3_frst {
            if self.divider_load_settle {
                // First ch3_2mhz↑ after ch3_restart↓ — DFFs settle out
                // of level-sensitive load mode but don't count yet.
                self.divider_load_settle = false;
            } else if self.frequency_timer > 0 {
                self.frequency_timer -= 1;
                if self.frequency_timer == 0 {
                    // Overflow → ch3_frst↑, reload, wave-position advance.
                    self.frequency_timer = 2048 - self.period.0 as u16;
                    self.wave_position = (self.wave_position + 1) % 32;
                    self.ch3_frst = true;
                }
            }
        }
    }

    /// Effects of `ch3_restart↑` (§14.8.6): wave-position counter
    /// async-reset via etan↓, divider load window opens via hera↓,
    /// and — if `wave_data_latch` is still high from a prior overflow
    /// — wave-RAM byte-0 (or 4-byte block) corruption per §14.8.5.
    /// Releases happen on the gyta-driven async-reset path.
    fn on_ch3_restart_rise(&mut self) {
        // DMG wave-RAM corruption per §14.8.5 (FST-anchored 2026-05-24
        // under `SIMPLIFIED_WAVERAM=`): ch3_restart↑ while the SRAM
        // bit-line precharge window (azus | azet) is open drives a
        // 4-byte ROW copy — `ram[0..3] ← ram[row*4..]` where
        // `row = wave_position >> 3`. Source row 0 naturally no-ops
        // (ram[i] = ram[i]) — no `byte_pos < 4` special case. Pan
        // Docs's single-byte framing is incorrect.
        if self.azus || self.azet {
            let row = (self.wave_position as usize) >> 3;
            let src = row * 4;
            for i in 0..4 {
                self.ram[i] = self.ram[src + i];
            }
        }

        // Reset wave-position counter (etan↓ async-resets all 5 cells).
        self.wave_position = 0;

        // Divider load window opens: counter level-sensitive on period.
        self.frequency_timer = 2048 - self.period.0 as u16;
        // ch3_frst is async-cleared too; wave_data_latch chain follows.
        self.ch3_frst = false;
    }

    /// On `ch3_restart↓` (the gyta-driven self-clear): the divider
    /// exits load mode. Set `divider_load_settle` so the very next
    /// `ch3_2mhz↑` is consumed by the DFFs' transition (no count yet);
    /// the count begins on the second `ch3_2mhz↑` after release.
    fn on_ch3_restart_fall(&mut self) {
        self.divider_load_settle = true;
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
    /// DMG wave-RAM bus alignment per §14.8.4. CPU access succeeds
    /// only while `wave_data_latch` (= AZUS) is high — its ~1-T-cycle
    /// pulse, ~1.5 T-cycles after `ch3_frst↑`. Accesses outside that
    /// window see a floating bus → 0xFF. `bus_value_at_latch`
    /// re-evaluates so accesses landing late in the M-cycle still
    /// see windows that opened after the drive-enable snapshot. The
    /// wider `(azus | azet)` SRAM-read-active window is only consulted
    /// by the corruption gate, not by CPU access — the CPU bus drive
    /// is gated by `wave_data_latch` proper, not by `azet`'s hold.
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
