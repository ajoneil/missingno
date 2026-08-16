//! Mapping the captured subsystem snapshots onto the schema-keyed record.

use missingno_core::state::StateRecord;

use super::{
    capture_apu, capture_cpu, capture_dma, capture_mbc, capture_ppu, capture_serial, capture_timer,
};
use crate::Console;

/// Read the whole console into a schema-keyed record. The pipeline latch fields
/// (nullable in the schema) are omitted — this is a boundary capture.
pub fn read_shared_record<M: crate::Model>(gb: &Console<M>) -> StateRecord {
    let cpu = capture_cpu(gb);
    let ppu = capture_ppu(gb);
    let apu = capture_apu(gb);
    let timer = capture_timer(gb);
    let dma = capture_dma(gb);
    let serial = capture_serial(gb);
    let mbc = capture_mbc(gb);

    let mut r = StateRecord::new();
    // CPU.
    r.set("a", cpu.a)
        .set("f", cpu.f)
        .set("b", cpu.b)
        .set("c", cpu.c)
        .set("d", cpu.d)
        .set("e", cpu.e)
        .set("h", cpu.h)
        .set("l", cpu.l)
        .set("sp", cpu.sp)
        .set("pc", cpu.pc)
        .set("ime", cpu.ime)
        .set("if_", cpu.if_)
        .set("ie", cpu.ie)
        .set("cpu_mode", cpu.halt_state)
        .set("ime_enable_pending", cpu.ei_delay != 0)
        .set("halt_bug", cpu.halt_bug);
    // PPU registers + deep boundary state.
    r.set("lcdc", ppu.lcdc)
        .set("stat", ppu.stat)
        .set("ly", ppu.ly)
        .set("lyc", ppu.lyc)
        .set("scy", ppu.scy)
        .set("scx", ppu.scx)
        .set("wy", ppu.wy)
        .set("wx", ppu.wx)
        .set("bgp", ppu.bgp)
        .set("obp0", ppu.obp0)
        .set("obp1", ppu.obp1)
        .set("dma", ppu.dma)
        .set("lx", ppu.dot_position)
        .set("stat_line", ppu.stat_line_was_high)
        .set("window_line_counter", ppu.window_line_counter);
    // Timer.
    r.set("div", timer.div)
        .set("tima", timer.tima)
        .set("tma", timer.tma)
        .set("tac", timer.tac)
        .set("internal_counter", timer.internal_counter)
        .set("overflow_pending", timer.overflow_pending)
        .set("reloading", timer.reloading);
    // Serial.
    r.set("sb", serial.sb)
        .set("sc", serial.sc)
        .set("serial_bits_remaining", serial.bits_remaining)
        .set("serial_clock", serial.shift_clock);
    // APU registers.
    r.set("master_vol", apu.master_vol)
        .set("sound_pan", apu.sound_pan)
        .set("sound_on", apu.sound_on)
        .set("ch1_sweep", apu.ch1_sweep)
        .set("ch1_duty_len", apu.ch1_duty_len)
        .set("ch1_vol_env", apu.ch1_vol_env)
        .set("ch1_freq_lo", apu.ch1_freq_lo)
        .set("ch1_freq_hi", apu.ch1_freq_hi)
        .set("ch2_duty_len", apu.ch2_duty_len)
        .set("ch2_vol_env", apu.ch2_vol_env)
        .set("ch2_freq_lo", apu.ch2_freq_lo)
        .set("ch2_freq_hi", apu.ch2_freq_hi)
        .set("ch3_dac", apu.ch3_dac)
        .set("ch3_len", apu.ch3_len)
        .set("ch3_vol", apu.ch3_vol)
        .set("ch3_freq_lo", apu.ch3_freq_lo)
        .set("ch3_freq_hi", apu.ch3_freq_hi)
        .set("ch4_len", apu.ch4_len)
        .set("ch4_vol_env", apu.ch4_vol_env)
        .set("ch4_freq", apu.ch4_freq)
        .set("ch4_control", apu.ch4_control);
    // APU deep boundary state.
    r.set("frame_sequencer_step", apu.frame_sequencer_step)
        .set("prev_div_apu_bit", apu.prev_div_apu_bit)
        .set("ch1_period", apu.ch1_period)
        .set("ch1_envelope_timer", apu.ch1_envelope_timer)
        .set("ch1_sweep_timer", apu.ch1_sweep_timer)
        .set("ch1_sweep_enabled", apu.ch1_sweep_enabled)
        .set("ch1_sweep_negate_used", apu.ch1_sweep_negate_used)
        .set("ch1_length_enabled", apu.ch1_length_enabled)
        .set("ch2_period", apu.ch2_period)
        .set("ch2_envelope_timer", apu.ch2_envelope_timer)
        .set("ch2_length_enabled", apu.ch2_length_enabled)
        .set("ch3_period", apu.ch3_period)
        .set("ch3_length_enabled", apu.ch3_length_enabled)
        .set("ch4_envelope_timer", apu.ch4_envelope_timer)
        .set("ch4_length_enabled", apu.ch4_length_enabled);
    // OAM DMA engine.
    r.set("dma_active", dma.active)
        .set("dma_source", dma.source)
        .set("dma_byte_index", dma.byte_index)
        .set("dma_delay", dma.delay_remaining);
    // Cartridge mapper latches.
    r.set("mbc_type", mbc.mbc_type)
        .set("rom_bank", mbc.rom_bank)
        .set("ram_bank", mbc.ram_bank)
        .set("ram_enabled", mbc.ram_enabled)
        .set("mbc_mode", mbc.mode);
    // Mapper-specific latches beyond the shared quadruple, set only for the
    // mapper that holds them (the schema fields are nullable — a mapper without
    // them omits them).
    if let Some(code) = mbc.clock_register {
        r.set("mbc3_clock_sel", code);
    }
    if let Some(rtc) = mbc.rtc {
        r.set("rtc_seconds", rtc.seconds)
            .set("rtc_minutes", rtc.minutes)
            .set("rtc_hours", rtc.hours)
            .set("rtc_day_lower", rtc.day_lower)
            .set("rtc_day_upper", rtc.day_upper)
            .set("rtc_latched_seconds", rtc.latched_seconds)
            .set("rtc_latched_minutes", rtc.latched_minutes)
            .set("rtc_latched_hours", rtc.latched_hours)
            .set("rtc_latched_day_lower", rtc.latched_day_lower)
            .set("rtc_latched_day_upper", rtc.latched_day_upper)
            .set("rtc_latch_ready", rtc.latch_ready);
    }
    if let Some(m6) = mbc.mbc6 {
        r.set("mbc6_rom_bank_b", m6.rom_bank_b)
            .set("mbc6_ram_bank_b", m6.ram_bank_b)
            .set("mbc6_rom_a_flash", m6.rom_a_flash)
            .set("mbc6_rom_b_flash", m6.rom_b_flash)
            .set("mbc6_flash_enabled", m6.flash_enabled);
    }
    if let Some(m7) = mbc.mbc7 {
        r.set("mbc7_ram_enabled_1", m7.ram_enabled_1)
            .set("mbc7_ram_enabled_2", m7.ram_enabled_2)
            .set("mbc7_accel_x", m7.accel_x)
            .set("mbc7_accel_y", m7.accel_y)
            .set("mbc7_write_enabled", m7.write_enabled);
    }
    r
}
