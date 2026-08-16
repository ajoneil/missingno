//! The DMG save-state bridge: it maps the Game Boy's internal state onto the
//! hardware-named [`SystemStateSchema`] and back. Capture reads the console into
//! a [`StateRecord`] keyed by the schema's field names; restore parses a record
//! into per-subsystem snapshot structs and rebuilds the console in place at an
//! instruction boundary.
//!
//! The snapshot structs are missingno's internal boundary DTO — the input the
//! per-component `from_snapshot` constructors already consume. The mapping
//! between a struct's emulator-shaped field and its hardware-named schema field
//! (halt_state → cpu_mode, ei_delay → ime_enable_pending, dot_position → lx) is
//! the bridge's substance; the format sees only the hardware quantity.
//!
//! Restore is boundary-faithful (Tier-2a): the pixel-pipeline latches are not
//! round-tripped — at a frame/instruction boundary they reconstruct from the
//! pipeline's boundary defaults — so the schema marks those fields nullable and
//! this bridge omits them.

mod capture;
mod parse;
mod record;
mod restore;

pub use capture::{
    capture_apu, capture_cpu, capture_dma, capture_mbc, capture_memory, capture_ppu,
    capture_serial, capture_timer,
};
pub use parse::parse_record;
pub use record::read_shared_record;

// ── Per-subsystem boundary snapshot structs ──────────────────────
//
// missingno's internal boundary DTO. Each per-component `from_snapshot`
// constructor consumes its struct; capture fills them from the console.

pub struct CpuSnapshot {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
    pub ime: bool,
    pub if_: u8,
    pub ie: u8,
    /// Run state code: 0 running, 1 halting, 2 halted/stopped, 3 locked.
    pub halt_state: u8,
    /// EI's deferred enable in flight (1) or not (0).
    pub ei_delay: u8,
    pub halt_bug: bool,
}

pub struct PpuSnapshot {
    pub lcdc: u8,
    pub stat: u8,
    pub ly: u8,
    pub lyc: u8,
    pub scy: u8,
    pub scx: u8,
    pub wy: u8,
    pub wx: u8,
    pub bgp: u8,
    pub obp0: u8,
    pub obp1: u8,
    pub dma: u8,
    pub dot_position: u8,
    pub stat_line_was_high: bool,
    pub window_line_counter: u8,
}

pub struct ApuSnapshot {
    pub master_vol: u8,
    pub sound_pan: u8,
    pub sound_on: u8,
    pub ch1_sweep: u8,
    pub ch1_duty_len: u8,
    pub ch1_vol_env: u8,
    pub ch1_freq_lo: u8,
    pub ch1_freq_hi: u8,
    pub ch2_duty_len: u8,
    pub ch2_vol_env: u8,
    pub ch2_freq_lo: u8,
    pub ch2_freq_hi: u8,
    pub ch3_dac: u8,
    pub ch3_len: u8,
    pub ch3_vol: u8,
    pub ch3_freq_lo: u8,
    pub ch3_freq_hi: u8,
    pub ch4_len: u8,
    pub ch4_vol_env: u8,
    pub ch4_freq: u8,
    pub ch4_control: u8,
    pub frame_sequencer_step: u8,
    pub prev_div_apu_bit: bool,
    pub ch1_period: u16,
    pub ch1_envelope_timer: u8,
    pub ch1_sweep_timer: u8,
    pub ch1_sweep_enabled: bool,
    pub ch1_sweep_negate_used: bool,
    pub ch1_length_enabled: bool,
    pub ch2_period: u16,
    pub ch2_envelope_timer: u8,
    pub ch2_length_enabled: bool,
    pub ch3_period: u16,
    pub ch3_length_enabled: bool,
    pub ch4_envelope_timer: u8,
    pub ch4_length_enabled: bool,
}

pub struct TimerSnapshot {
    pub div: u8,
    pub tima: u8,
    pub tma: u8,
    pub tac: u8,
    pub internal_counter: u16,
    pub overflow_pending: bool,
    pub reloading: bool,
}

pub struct DmaSnapshot {
    pub active: bool,
    pub source: u16,
    pub byte_index: u8,
    pub delay_remaining: u8,
}

pub struct SerialSnapshot {
    pub sb: u8,
    pub sc: u8,
    pub bits_remaining: u8,
    pub shift_clock: bool,
}

pub struct MbcSnapshot {
    pub mbc_type: String,
    pub rom_bank: u16,
    pub ram_bank: u8,
    pub ram_enabled: bool,
    pub mode: u8,
    /// MBC3: `Some(0..=4)` when a clock register is mapped at `$A000` instead of
    /// RAM (index Seconds..DayUpper); `None` when RAM is mapped. Absent for
    /// every other mapper.
    pub clock_register: Option<u8>,
    /// MBC3 real-time clock, on carts that carry one.
    pub rtc: Option<RtcRegs>,
    /// MBC6's second ROM/RAM half and flash latches.
    pub mbc6: Option<Mbc6State>,
    /// MBC7's split enables, accelerometer latch, and EEPROM write-enable.
    pub mbc7: Option<Mbc7State>,
}

/// MBC3 real-time-clock register file as saved: the live counters ($08-$0C), the
/// latched copies a $6000 sequence froze, and the arm flag. RTCDH (day-upper)
/// bit 6 is the halt, bit 7 the sticky day-counter carry.
#[derive(Clone, Copy)]
pub struct RtcRegs {
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    pub day_lower: u8,
    pub day_upper: u8,
    pub latched_seconds: u8,
    pub latched_minutes: u8,
    pub latched_hours: u8,
    pub latched_day_lower: u8,
    pub latched_day_upper: u8,
    pub latch_ready: bool,
}

/// MBC6's second switchable half and flash state — the fields the shared
/// rom_bank/ram_bank/ram_enabled triple does not carry.
#[derive(Clone, Copy)]
pub struct Mbc6State {
    pub rom_bank_b: u8,
    pub ram_bank_b: u8,
    pub rom_a_flash: bool,
    pub rom_b_flash: bool,
    pub flash_enabled: bool,
}

/// MBC7's two independent RAM-enable latches, the latched accelerometer axes,
/// and the EEPROM write-enable.
#[derive(Clone, Copy)]
pub struct Mbc7State {
    pub ram_enabled_1: bool,
    pub ram_enabled_2: bool,
    pub accel_x: u16,
    pub accel_y: u16,
    pub write_enabled: bool,
}

/// The clock register a $4000-$5FFF select code names, as a save-state index.
fn clock_register_code(register: crate::cartridge::mbc::mbc3::ClockRegister) -> u8 {
    use crate::cartridge::mbc::mbc3::ClockRegister::*;
    match register {
        Seconds => 0,
        Minutes => 1,
        Hours => 2,
        DayLower => 3,
        DayUpper => 4,
    }
}

/// The clock register a saved index names; out-of-range falls back to Seconds.
fn clock_register_from_code(code: u8) -> crate::cartridge::mbc::mbc3::ClockRegister {
    use crate::cartridge::mbc::mbc3::ClockRegister::*;
    match code {
        1 => Minutes,
        2 => Hours,
        3 => DayLower,
        4 => DayUpper,
        _ => Seconds,
    }
}

/// All parsed subsystem snapshots plus the named RAM regions — the input to an
/// in-place restore.
pub struct Snapshot {
    pub cpu: CpuSnapshot,
    pub ppu: PpuSnapshot,
    pub apu: ApuSnapshot,
    pub timer: TimerSnapshot,
    pub dma: DmaSnapshot,
    pub serial: SerialSnapshot,
    pub mbc: MbcSnapshot,
    pub memory: Vec<(String, Vec<u8>)>,
}
