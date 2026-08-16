//! Reading a schema-keyed record back into the boundary snapshot structs.

use missingno_core::state::{StateRecord, StateValue};
use missingno_core::system::StateError;

use super::{
    ApuSnapshot, CpuSnapshot, DmaSnapshot, Mbc6State, Mbc7State, MbcSnapshot, PpuSnapshot, RtcRegs,
    SerialSnapshot, Snapshot, TimerSnapshot,
};

fn int(record: &StateRecord, name: &str) -> Result<u32, StateError> {
    match record.get(name) {
        Some(StateValue::Int(v)) => Ok(*v),
        _ => Err(StateError::Corrupt),
    }
}

fn u8_of(record: &StateRecord, name: &str) -> Result<u8, StateError> {
    Ok(int(record, name)? as u8)
}

fn u16_of(record: &StateRecord, name: &str) -> Result<u16, StateError> {
    Ok(int(record, name)? as u16)
}

fn bool_of(record: &StateRecord, name: &str) -> Result<bool, StateError> {
    match record.get(name) {
        Some(StateValue::Bool(b)) => Ok(*b),
        _ => Err(StateError::Corrupt),
    }
}

fn text_of(record: &StateRecord, name: &str) -> Result<String, StateError> {
    match record.get(name) {
        Some(StateValue::Text(t)) => Ok(t.clone()),
        _ => Err(StateError::Corrupt),
    }
}

/// A nullable u8 field: `None` when the record omits it or carries it as null.
fn opt_u8(record: &StateRecord, name: &str) -> Result<Option<u8>, StateError> {
    match record.get(name) {
        Some(StateValue::Int(v)) if *v <= u8::MAX as u32 => Ok(Some(*v as u8)),
        Some(StateValue::Null) | None => Ok(None),
        _ => Err(StateError::Corrupt),
    }
}

/// Parse the MBC3 real-time clock, present only when the record carries it.
fn parse_rtc(record: &StateRecord) -> Result<Option<RtcRegs>, StateError> {
    if opt_u8(record, "rtc_seconds")?.is_none() {
        return Ok(None);
    }
    Ok(Some(RtcRegs {
        seconds: u8_of(record, "rtc_seconds")?,
        minutes: u8_of(record, "rtc_minutes")?,
        hours: u8_of(record, "rtc_hours")?,
        day_lower: u8_of(record, "rtc_day_lower")?,
        day_upper: u8_of(record, "rtc_day_upper")?,
        latched_seconds: u8_of(record, "rtc_latched_seconds")?,
        latched_minutes: u8_of(record, "rtc_latched_minutes")?,
        latched_hours: u8_of(record, "rtc_latched_hours")?,
        latched_day_lower: u8_of(record, "rtc_latched_day_lower")?,
        latched_day_upper: u8_of(record, "rtc_latched_day_upper")?,
        latch_ready: bool_of(record, "rtc_latch_ready")?,
    }))
}

/// Parse MBC6's second-half/flash state, present only for an MBC6 save.
fn parse_mbc6(record: &StateRecord) -> Result<Option<Mbc6State>, StateError> {
    if opt_u8(record, "mbc6_rom_bank_b")?.is_none() {
        return Ok(None);
    }
    Ok(Some(Mbc6State {
        rom_bank_b: u8_of(record, "mbc6_rom_bank_b")?,
        ram_bank_b: u8_of(record, "mbc6_ram_bank_b")?,
        rom_a_flash: bool_of(record, "mbc6_rom_a_flash")?,
        rom_b_flash: bool_of(record, "mbc6_rom_b_flash")?,
        flash_enabled: bool_of(record, "mbc6_flash_enabled")?,
    }))
}

/// Parse MBC7's latches, present only for an MBC7 save.
fn parse_mbc7(record: &StateRecord) -> Result<Option<Mbc7State>, StateError> {
    match record.get("mbc7_ram_enabled_1") {
        Some(StateValue::Bool(_)) => {}
        _ => return Ok(None),
    }
    Ok(Some(Mbc7State {
        ram_enabled_1: bool_of(record, "mbc7_ram_enabled_1")?,
        ram_enabled_2: bool_of(record, "mbc7_ram_enabled_2")?,
        accel_x: u16_of(record, "mbc7_accel_x")?,
        accel_y: u16_of(record, "mbc7_accel_y")?,
        write_enabled: bool_of(record, "mbc7_write_enabled")?,
    }))
}

/// Parse a validated record (and the file's memory spans) into the boundary
/// snapshot structs an in-place restore consumes.
pub fn parse_record(
    record: &StateRecord,
    memory: Vec<(String, Vec<u8>)>,
) -> Result<Snapshot, StateError> {
    let cpu = CpuSnapshot {
        a: u8_of(record, "a")?,
        f: u8_of(record, "f")?,
        b: u8_of(record, "b")?,
        c: u8_of(record, "c")?,
        d: u8_of(record, "d")?,
        e: u8_of(record, "e")?,
        h: u8_of(record, "h")?,
        l: u8_of(record, "l")?,
        sp: u16_of(record, "sp")?,
        pc: u16_of(record, "pc")?,
        ime: bool_of(record, "ime")?,
        if_: u8_of(record, "if_")?,
        ie: u8_of(record, "ie")?,
        halt_state: u8_of(record, "cpu_mode")?,
        ei_delay: bool_of(record, "ime_enable_pending")? as u8,
        halt_bug: bool_of(record, "halt_bug")?,
    };
    let ppu = PpuSnapshot {
        lcdc: u8_of(record, "lcdc")?,
        stat: u8_of(record, "stat")?,
        ly: u8_of(record, "ly")?,
        lyc: u8_of(record, "lyc")?,
        scy: u8_of(record, "scy")?,
        scx: u8_of(record, "scx")?,
        wy: u8_of(record, "wy")?,
        wx: u8_of(record, "wx")?,
        bgp: u8_of(record, "bgp")?,
        obp0: u8_of(record, "obp0")?,
        obp1: u8_of(record, "obp1")?,
        dma: u8_of(record, "dma")?,
        dot_position: u8_of(record, "lx")?,
        stat_line_was_high: bool_of(record, "stat_line")?,
        window_line_counter: u8_of(record, "window_line_counter")?,
    };
    let apu = ApuSnapshot {
        master_vol: u8_of(record, "master_vol")?,
        sound_pan: u8_of(record, "sound_pan")?,
        sound_on: u8_of(record, "sound_on")?,
        ch1_sweep: u8_of(record, "ch1_sweep")?,
        ch1_duty_len: u8_of(record, "ch1_duty_len")?,
        ch1_vol_env: u8_of(record, "ch1_vol_env")?,
        ch1_freq_lo: u8_of(record, "ch1_freq_lo")?,
        ch1_freq_hi: u8_of(record, "ch1_freq_hi")?,
        ch2_duty_len: u8_of(record, "ch2_duty_len")?,
        ch2_vol_env: u8_of(record, "ch2_vol_env")?,
        ch2_freq_lo: u8_of(record, "ch2_freq_lo")?,
        ch2_freq_hi: u8_of(record, "ch2_freq_hi")?,
        ch3_dac: u8_of(record, "ch3_dac")?,
        ch3_len: u8_of(record, "ch3_len")?,
        ch3_vol: u8_of(record, "ch3_vol")?,
        ch3_freq_lo: u8_of(record, "ch3_freq_lo")?,
        ch3_freq_hi: u8_of(record, "ch3_freq_hi")?,
        ch4_len: u8_of(record, "ch4_len")?,
        ch4_vol_env: u8_of(record, "ch4_vol_env")?,
        ch4_freq: u8_of(record, "ch4_freq")?,
        ch4_control: u8_of(record, "ch4_control")?,
        frame_sequencer_step: u8_of(record, "frame_sequencer_step")?,
        prev_div_apu_bit: bool_of(record, "prev_div_apu_bit")?,
        ch1_period: u16_of(record, "ch1_period")?,
        ch1_envelope_timer: u8_of(record, "ch1_envelope_timer")?,
        ch1_sweep_timer: u8_of(record, "ch1_sweep_timer")?,
        ch1_sweep_enabled: bool_of(record, "ch1_sweep_enabled")?,
        ch1_sweep_negate_used: bool_of(record, "ch1_sweep_negate_used")?,
        ch1_length_enabled: bool_of(record, "ch1_length_enabled")?,
        ch2_period: u16_of(record, "ch2_period")?,
        ch2_envelope_timer: u8_of(record, "ch2_envelope_timer")?,
        ch2_length_enabled: bool_of(record, "ch2_length_enabled")?,
        ch3_period: u16_of(record, "ch3_period")?,
        ch3_length_enabled: bool_of(record, "ch3_length_enabled")?,
        ch4_envelope_timer: u8_of(record, "ch4_envelope_timer")?,
        ch4_length_enabled: bool_of(record, "ch4_length_enabled")?,
    };
    let timer = TimerSnapshot {
        div: u8_of(record, "div")?,
        tima: u8_of(record, "tima")?,
        tma: u8_of(record, "tma")?,
        tac: u8_of(record, "tac")?,
        internal_counter: u16_of(record, "internal_counter")?,
        overflow_pending: bool_of(record, "overflow_pending")?,
        reloading: bool_of(record, "reloading")?,
    };
    let dma = DmaSnapshot {
        active: bool_of(record, "dma_active")?,
        source: u16_of(record, "dma_source")?,
        byte_index: u8_of(record, "dma_byte_index")?,
        delay_remaining: u8_of(record, "dma_delay")?,
    };
    let serial = SerialSnapshot {
        sb: u8_of(record, "sb")?,
        sc: u8_of(record, "sc")?,
        bits_remaining: u8_of(record, "serial_bits_remaining")?,
        shift_clock: bool_of(record, "serial_clock")?,
    };
    let mbc = MbcSnapshot {
        mbc_type: text_of(record, "mbc_type")?,
        rom_bank: u16_of(record, "rom_bank")?,
        ram_bank: u8_of(record, "ram_bank")?,
        ram_enabled: bool_of(record, "ram_enabled")?,
        mode: u8_of(record, "mbc_mode")?,
        clock_register: opt_u8(record, "mbc3_clock_sel")?,
        rtc: parse_rtc(record)?,
        mbc6: parse_mbc6(record)?,
        mbc7: parse_mbc7(record)?,
    };
    Ok(Snapshot {
        cpu,
        ppu,
        apu,
        timer,
        dma,
        serial,
        mbc,
        memory,
    })
}
