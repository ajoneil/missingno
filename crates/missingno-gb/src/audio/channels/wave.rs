use super::super::Audio;
use super::{
    Enabled,
    registers::{PeriodHighAndControl, Signed11},
};

/// How the CPU couples to CH3's wave SRAM while the channel is active.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WaveRamCoupling {
    /// DMG: the CPU reaches the SRAM only during CH3's own fetch
    /// strobe (BUSA/AZUS window); outside it the bus floats to 0xFF,
    /// and a retrigger inside it shorts wordlines (row corruption).
    FetchStrobe,
    /// CGB: arbitration grants the CPU the channel's current byte
    /// unconditionally; no float window, no retrigger corruption.
    ChannelPosition,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Register {
    Volume,
    DacEnabled,
    Length,
    PeriodLow,
    PeriodHighAndControl,
}

/// NR34 trigger 4-stage synchroniser: `gavu → foba → gara → gyta`.
/// Each NR34 write with bit 7 set enters at `bit_latch`, propagates
/// through the M-cycle-boundary and ch3_2mhz↓ samplers, and emerges
/// as a one-`ch3_2mhz`-cycle pulse on `restart` that drives the
/// divider load network.
#[derive(Clone, Default)]
pub struct TriggerSync {
    /// `gavu` — NR34 d7 captured at apu_wr ↑.
    pub bit_latch: bool,
    /// `foba` — bit_latch synced to the M-cycle boundary
    /// (`apu_phi ↑` = T=0 rise of M+1).
    pub armed: bool,
    /// `gara` — pulse fires on the next fabo ↑ (= ch3_2mhz ↓) when
    /// `armed` is high; held high for one ch3_2mhz cycle.
    pub restart: bool,
    /// `gyta` — samples `restart` on fabo ↑ and on the following
    /// fabo ↑ async-resets `gara` via `fury`.
    pub self_clear: bool,
}

/// 3-stage apu_4mhz synchroniser from `ch3_frst` to the
/// `wave_data_latch` strobe, plus the AZET extension that holds the
/// prior T-cycle's latched value.
#[derive(Clone, Default)]
pub struct WaveDataLatch {
    /// `busa` — captures `ch3_frst` on apu_4mhz ↑.
    pub sync_1: bool,
    /// `bano` — captures `busa` on cozy ↑ (= our rise edge).
    pub sync_2: bool,
    /// `azus` — captures `bano` on apu_4mhz ↑. THIS is the
    /// `wave_data_latch` strobe: CPU reads of FF30..FF3F return the
    /// wave-RAM byte while it's high; outside the window the bus
    /// floats to 0xFF.
    pub latched: bool,
    /// `azet` — captures `latched` on apu_4mhz ↓ to extend the
    /// SRAM-read-active window by one T-cycle. `(latched | extended)
    /// = 1` keeps `wave_ram_bl_precharge = 0` and is the corruption
    /// gate evaluated at `restart ↑`.
    pub extended: bool,
}

#[derive(Clone)]
pub struct WaveChannel {
    pub enabled: Enabled,
    pub dac_enabled: bool,
    pub volume: Volume,
    pub length_enabled: bool,
    pub period: Signed11,
    pub ram: [u8; 16],
    pub length_counter: u16,

    /// `cery` /2 prescaler — toggles on every master-clock rise;
    /// held at 0 while `apu_reset = 1`.
    pub ch3_2mhz: bool,
    /// Divider down-counter; reloads `(2048 - period)` on each
    /// `ch3_frst ↑`, held at the loaded value while
    /// `trigger_sync.restart` is high (load mode via
    /// `hera = NOR(ch3_frst, ch3_restart)`).
    pub frequency_timer: u16,
    pub wave_position: u8,
    /// NAND-latch gating `juty` (the divider toggle clock). Set by
    /// DAC-off or `apu_reset`; cleared by the gyta-derived `s_n`
    /// pulse on a trigger. While set, no divider overflows fire.
    pub ch3_fdis: bool,
    /// `huno` output — overflow capture pulse, held one ch3_2mhz
    /// cycle and cleared by `hupa = AND(huno, ch3_2mhz)`.
    pub ch3_frst: bool,
    /// `huno` DFF clock-to-q ripple — one master-clock edge between
    /// divider wrap and `ch3_frst ↑`.
    pub pending_overflow: bool,

    pub trigger_sync: TriggerSync,
    pub wave_data_latch: WaveDataLatch,
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
            length_counter: 0,

            ch3_2mhz: false,
            frequency_timer: 0,
            wave_position: 0,
            ch3_fdis: true,
            ch3_frst: false,
            pending_overflow: false,

            trigger_sync: TriggerSync::default(),
            wave_data_latch: WaveDataLatch::default(),
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
            length_counter,

            ch3_2mhz: false,
            frequency_timer: 0,
            wave_position: 0,
            ch3_fdis: true,
            ch3_frst: false,
            pending_overflow: false,

            trigger_sync: TriggerSync::default(),
            wave_data_latch: WaveDataLatch::default(),
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

    pub fn write_register(&mut self, register: Register, value: u8, caru_low: bool) {
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

                // doda = NOR(fugo, bufy_256hz, ff23_d6_n): length-enable
                // 0→1 rises doda (one extra length count) iff caru is low.
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
                    if caru_low && self.length_enabled && self.length_counter == 256 {
                        self.length_counter = 255;
                    }
                }
            }
        }
    }

    pub fn trigger(&mut self) {
        // `gavu` is a level-sensitive drlatch; capture from commit_write
        // is equivalent to hardware's apu_wr↑ capture.
        self.trigger_sync.bit_latch = true;

        self.enabled.enabled = true;
        if self.length_counter == 0 {
            self.length_counter = 256;
        }

        if !self.dac_enabled {
            self.enabled.enabled = false;
        }
    }

    pub fn tcycle(&mut self, t_index: u8, apu_reset_n: bool, coupling: WaveRamCoupling) {
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
        // `(latched | extended)` SRAM-read-active window spans 1.5
        // T-cycles.
        self.wave_data_latch.sync_2 = self.wave_data_latch.sync_1;
        self.wave_data_latch.extended = self.wave_data_latch.latched;

        // foba captures gavu on apu_phi↑ (= M-cycle boundary, T=0).
        if t_index == 0 {
            self.trigger_sync.armed = self.trigger_sync.bit_latch;
        }

        if fabo_rising {
            // gara/gyta sample on fabo↑. gyta captures the prior
            // `restart`; when high it async-resets gara via fury.
            let new_self_clear = self.trigger_sync.restart;
            if new_self_clear {
                self.trigger_sync.restart = false;
                self.trigger_sync.armed = false;
                self.trigger_sync.bit_latch = false;
                self.trigger_sync.self_clear = true;
                self.on_ch3_restart_fall();
            } else if self.trigger_sync.armed {
                self.trigger_sync.self_clear = false;
                self.trigger_sync.restart = true;
                self.on_ch3_restart_rise(coupling);
            } else {
                self.trigger_sync.self_clear = false;
            }
        }

        // hupa = AND(huno, ch3_2mhz): clears ch3_frst on the next
        // ch3_2mhz↑. The wave-position counter advances on this edge
        // (= dero↑), one ch3_2mhz cycle after the overflow.
        if ch3_2mhz_rising && self.ch3_frst {
            self.ch3_frst = false;
            self.wave_position = (self.wave_position + 1) % 32;
        }

        // Divider clocks on ch3_2mhz↑ when hera is high (= restart
        // and ch3_frst both low — out of load mode) and juty is active
        // (= ch3_fdis = 0, channel enabled).
        if ch3_2mhz_rising
            && !self.trigger_sync.restart
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
    /// bit-lines are still driven (DMG coupling only), then
    /// wave-position async-reset and divider load.
    fn on_ch3_restart_rise(&mut self, coupling: WaveRamCoupling) {
        // Retrigger-during-active-read shorts SRAM wordlines, copying
        // part of CH3's currently-read row into ram[0..3]. With
        // byte_pos < 4 (source row == destination row 0) only one
        // cell shorts, so just ram[0] gets ram[byte_pos]. With
        // byte_pos >= 4, four column wordlines are enabled across
        // both rows and the full 4-byte row copies through.
        if coupling == WaveRamCoupling::FetchStrobe
            && (self.wave_data_latch.latched || self.wave_data_latch.extended)
        {
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
    /// wave_data_latch chain that clock on apu_4mhz ↑.
    pub fn fall_sync(&mut self) {
        self.wave_data_latch.latched = self.wave_data_latch.sync_2;
        self.wave_data_latch.sync_1 = self.ch3_frst;
    }

    pub fn tick_length(&mut self) {
        if self.length_enabled && self.length_counter > 0 {
            self.length_counter -= 1;
            if self.length_counter == 0 {
                self.enabled.enabled = false;
            }
        }
    }

    pub fn digital_sample(&self) -> u8 {
        if !self.enabled.enabled {
            return 0;
        }
        let byte = self.ram[self.wave_position as usize / 2];
        let nibble = if self.wave_position.is_multiple_of(2) {
            byte >> 4
        } else {
            byte & 0x0f
        };
        match self.volume.shift() {
            0 => 0,
            shift => nibble >> (shift - 1),
        }
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
    /// position — under `FetchStrobe` coupling only while
    /// `wave_data_latch` is high (outside that ~1-T-cycle pulse the
    /// bus floats to 0xFF); under `ChannelPosition` coupling always.
    pub fn read_wave_ram(&self, offset: u8, coupling: WaveRamCoupling) -> u8 {
        let ch3 = &self.channels.ch3;
        if !ch3.enabled.enabled {
            return ch3.ram[offset as usize];
        }
        if coupling == WaveRamCoupling::ChannelPosition || ch3.wave_data_latch.latched {
            ch3.ram[ch3.wave_position as usize / 2]
        } else {
            0xFF
        }
    }

    /// Active-channel writes target `ram[wave_position[4:1]]` (= the
    /// byte CH3 is reading). Under `FetchStrobe` coupling they only
    /// commit when the SRAM wordline driver is out of precharge: the
    /// `(latched | extended)` check at the T=3 fall commit edge covers
    /// both half-T edges of T=3 of the `wave_ram_wr` pulse. Under
    /// `ChannelPosition` coupling they always commit.
    pub fn write_wave_ram(&mut self, offset: u8, value: u8, coupling: WaveRamCoupling) {
        let ch3 = &mut self.channels.ch3;
        if !ch3.enabled.enabled {
            ch3.ram[offset as usize] = value;
            return;
        }
        if coupling == WaveRamCoupling::ChannelPosition
            || ch3.wave_data_latch.latched
            || ch3.wave_data_latch.extended
        {
            let byte_idx = ch3.wave_position as usize / 2;
            ch3.ram[byte_idx] = value;
        }
    }
}
