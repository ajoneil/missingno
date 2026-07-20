//! The Game Boy family's hardware state schema — the authored, hardware-named
//! description of DMG machine state the save-state bridge and trace writer key
//! their records on. This is DATA, not capture logic: it names the registers,
//! counters, and latches the silicon has, tiered by how observable each is.
//!
//! Tier 1 is the CPU-visible surface (registers + the IO register file + the
//! memory map + the framebuffer). Tier 2a is the boundary-complete deep state —
//! the pipeline latches, channel counters, and mapper latches needed to restore
//! at an instruction/frame boundary; it is a superset of what `snapshot.rs`
//! captures today. The CPU micro-sequencer phase and clock edge (Tier 2b) are
//! deliberately not named here — see the exclusions note below.
//!
//! `missingno-gbc` composes the CGB schema as these DMG fields plus its colour
//! delta, so the Tier-1/Tier-2a field builders are public.
//!
//! Excluded from this schema, by design:
//! - The CPU micro-sequencer (op state, M-cycle phase, bus/data latches) and
//!   dispatch DFFs are Tier 2b — legitimate hardware state, not yet named at the
//!   seam; arbitrary-tick restore awaits it.
//! - Per-step trace observations (pixel output, VRAM/APU write tracking, the
//!   mode-2 OAM sprite store, the sub-dot PPU divider signals) are trace-framing
//!   surfaces, re-derivable at a boundary — not machine state.

use std::sync::LazyLock;

use missingno_core::state::{
    FieldDef, FieldType, FrameSpec, MemorySpan, PixelFormat, SystemStateSchema,
};

use crate::frame::NATIVE_SIZE;

use FieldType::{Bool, Str, U8, U16};

/// Tier-1 observable fields: the CPU register file, the IO register file, and
/// the interrupt registers — the surface any emulator produces from hardware.
pub fn dmg_observable_fields() -> Vec<FieldDef> {
    vec![
        // CPU register file.
        FieldDef::observable("a", U8, "cpu"),
        FieldDef::observable("f", U8, "cpu").help("flags — Z N H C in bits 7..4"),
        FieldDef::observable("b", U8, "cpu"),
        FieldDef::observable("c", U8, "cpu"),
        FieldDef::observable("d", U8, "cpu"),
        FieldDef::observable("e", U8, "cpu"),
        FieldDef::observable("h", U8, "cpu"),
        FieldDef::observable("l", U8, "cpu"),
        FieldDef::observable("sp", U16, "cpu").help("stack pointer"),
        FieldDef::observable("pc", U16, "cpu").help("program counter"),
        FieldDef::observable("ime", Bool, "cpu").help("interrupt master enable"),
        FieldDef::observable("if_", U8, "cpu").help("IF ($FF0F) — pending interrupt flags"),
        FieldDef::observable("ie", U8, "cpu").help("IE ($FFFF) — interrupt enable mask"),
        // PPU register file.
        FieldDef::observable("lcdc", U8, "ppu").help("LCDC ($FF40)"),
        FieldDef::observable("stat", U8, "ppu").help("STAT ($FF41)"),
        FieldDef::observable("ly", U8, "ppu").help("LY ($FF44) — current scanline"),
        FieldDef::observable("lyc", U8, "ppu").help("LYC ($FF45)"),
        FieldDef::observable("scy", U8, "ppu").help("SCY ($FF42)"),
        FieldDef::observable("scx", U8, "ppu").help("SCX ($FF43)"),
        FieldDef::observable("wy", U8, "ppu").help("WY ($FF4A)"),
        FieldDef::observable("wx", U8, "ppu").help("WX ($FF4B)"),
        FieldDef::observable("bgp", U8, "ppu").help("BGP ($FF47)"),
        FieldDef::observable("obp0", U8, "ppu").help("OBP0 ($FF48)"),
        FieldDef::observable("obp1", U8, "ppu").help("OBP1 ($FF49)"),
        FieldDef::observable("dma", U8, "ppu").help("DMA ($FF46) — OAM DMA source register"),
        // Timer register file.
        FieldDef::observable("div", U8, "timer").help("DIV ($FF04) — top 8 bits of the counter"),
        FieldDef::observable("tima", U8, "timer").help("TIMA ($FF05)"),
        FieldDef::observable("tma", U8, "timer").help("TMA ($FF06)"),
        FieldDef::observable("tac", U8, "timer").help("TAC ($FF07)"),
        // Serial register file.
        FieldDef::observable("sb", U8, "serial").help("SB ($FF01)"),
        FieldDef::observable("sc", U8, "serial").help("SC ($FF02)"),
        // APU register file (includes write-only registers).
        FieldDef::observable("ch1_sweep", U8, "apu").help("NR10"),
        FieldDef::observable("ch1_duty_len", U8, "apu").help("NR11"),
        FieldDef::observable("ch1_vol_env", U8, "apu").help("NR12"),
        FieldDef::observable("ch1_freq_lo", U8, "apu").help("NR13"),
        FieldDef::observable("ch1_freq_hi", U8, "apu").help("NR14"),
        FieldDef::observable("ch2_duty_len", U8, "apu").help("NR21"),
        FieldDef::observable("ch2_vol_env", U8, "apu").help("NR22"),
        FieldDef::observable("ch2_freq_lo", U8, "apu").help("NR23"),
        FieldDef::observable("ch2_freq_hi", U8, "apu").help("NR24"),
        FieldDef::observable("ch3_dac", U8, "apu").help("NR30"),
        FieldDef::observable("ch3_len", U8, "apu").help("NR31"),
        FieldDef::observable("ch3_vol", U8, "apu").help("NR32"),
        FieldDef::observable("ch3_freq_lo", U8, "apu").help("NR33"),
        FieldDef::observable("ch3_freq_hi", U8, "apu").help("NR34"),
        FieldDef::observable("ch4_len", U8, "apu").help("NR41"),
        FieldDef::observable("ch4_vol_env", U8, "apu").help("NR42"),
        FieldDef::observable("ch4_freq", U8, "apu").help("NR43"),
        FieldDef::observable("ch4_control", U8, "apu").help("NR44"),
        FieldDef::observable("master_vol", U8, "apu").help("NR50"),
        FieldDef::observable("sound_pan", U8, "apu").help("NR51"),
        FieldDef::observable("sound_on", U8, "apu").help("NR52"),
    ]
}

/// Tier-2a boundary-complete deep state: the counters, latches, and pipeline
/// cells needed to restore at an instruction/frame boundary. A superset of
/// `snapshot.rs`, adding the pixel pipeline the lossy boundary restore skips.
pub fn dmg_boundary_fields() -> Vec<FieldDef> {
    vec![
        // CPU deep state. `cpu_mode` and `ime_enable_pending` name the hardware
        // quantity; the bridge maps missingno's model onto them (its Halting /
        // Locked sub-states, and its single EI shadow flag) — losslessly once
        // named at the hardware level.
        FieldDef::boundary("cpu_mode", U8, "cpu").help("run state — HALT / STOP versus running"),
        FieldDef::boundary("ime_enable_pending", Bool, "cpu")
            .help("EI's deferred enable in flight — IME sets after the next instruction"),
        FieldDef::boundary("halt_bug", Bool, "cpu")
            .help("HALT-bug latch — the byte after HALT re-reads because PC did not advance"),
        // PPU line/dot counters and edge-detect latch.
        FieldDef::boundary("lx", U8, "ppu").help("LX — dot position on the current line (0..113)"),
        FieldDef::boundary("stat_line", Bool, "ppu")
            .help("LALU.q — the STAT interrupt line's prior level, for the rising-edge detector"),
        FieldDef::boundary("window_line_counter", U8, "ppu").help("internal window line counter"),
        // PPU pixel pipeline: the two FIFOs, the palette pipe, the fetcher state
        // and its tile-row temporaries, and the per-line counters/flags. These
        // are nullable — a boundary save omits them: at a frame/instruction
        // boundary the pipeline is idle, so the restore reconstructs them from
        // the pipeline's boundary defaults. A mid-scanline (tick-complete)
        // producer fills them.
        FieldDef::boundary("bgw_fifo_a", U8, "ppu")
            .help("background/window FIFO, plane A")
            .nullable(),
        FieldDef::boundary("bgw_fifo_b", U8, "ppu")
            .help("background/window FIFO, plane B")
            .nullable(),
        FieldDef::boundary("spr_fifo_a", U8, "ppu")
            .help("sprite FIFO, plane A")
            .nullable(),
        FieldDef::boundary("spr_fifo_b", U8, "ppu")
            .help("sprite FIFO, plane B")
            .nullable(),
        FieldDef::boundary("pal_pipe", U8, "ppu")
            .help("palette pipeline")
            .nullable(),
        FieldDef::boundary("tfetch_state", U8, "ppu")
            .help("background fetcher state")
            .nullable(),
        FieldDef::boundary("sfetch_state", U8, "ppu")
            .help("sprite fetcher state")
            .nullable(),
        FieldDef::boundary("tile_temp_a", U8, "ppu")
            .help("fetched tile row, plane A")
            .nullable(),
        FieldDef::boundary("tile_temp_b", U8, "ppu")
            .help("fetched tile row, plane B")
            .nullable(),
        FieldDef::boundary("pix_count", U8, "ppu")
            .help("pixels pushed on this line")
            .nullable(),
        FieldDef::boundary("sprite_count", U8, "ppu")
            .help("sprites found for this line")
            .nullable(),
        FieldDef::boundary("scan_count", U8, "ppu")
            .help("OAM entries scanned")
            .nullable(),
        FieldDef::boundary("rendering", Bool, "ppu")
            .help("pixel pipeline active")
            .nullable(),
        FieldDef::boundary("win_mode", Bool, "ppu")
            .help("fetching the window rather than background")
            .nullable(),
        // APU divider + per-channel counters.
        FieldDef::boundary("frame_sequencer_step", U8, "apu").help("frame-sequencer step (0..7)"),
        FieldDef::boundary("prev_div_apu_bit", Bool, "apu")
            .help("prior DIV bit that clocks the frame sequencer"),
        FieldDef::boundary("ch1_period", U16, "apu"),
        FieldDef::boundary("ch1_envelope_timer", U8, "apu"),
        FieldDef::boundary("ch1_sweep_timer", U8, "apu"),
        FieldDef::boundary("ch1_sweep_enabled", Bool, "apu"),
        FieldDef::boundary("ch1_sweep_negate_used", Bool, "apu")
            .help("negate mode has been used since the last trigger"),
        FieldDef::boundary("ch1_length_enabled", Bool, "apu"),
        FieldDef::boundary("ch2_period", U16, "apu"),
        FieldDef::boundary("ch2_envelope_timer", U8, "apu"),
        FieldDef::boundary("ch2_length_enabled", Bool, "apu"),
        FieldDef::boundary("ch3_period", U16, "apu"),
        FieldDef::boundary("ch3_length_enabled", Bool, "apu"),
        FieldDef::boundary("ch4_envelope_timer", U8, "apu"),
        FieldDef::boundary("ch4_length_enabled", Bool, "apu"),
        // Timer internals.
        FieldDef::boundary("internal_counter", U16, "timer")
            .help("full 16-bit counter; DIV exposes its top 8 bits"),
        FieldDef::boundary("overflow_pending", Bool, "timer")
            .help("TIMA overflowed; TMA reload happens next M-cycle"),
        FieldDef::boundary("reloading", Bool, "timer")
            .help("TIMA is reloading from TMA this M-cycle"),
        // OAM DMA engine.
        FieldDef::boundary("dma_active", Bool, "dma").help("an OAM DMA transfer is running"),
        FieldDef::boundary("dma_source", U16, "dma").help("OAM DMA source base address"),
        FieldDef::boundary("dma_byte_index", U8, "dma").help("bytes copied so far (0..159)"),
        FieldDef::boundary("dma_delay", U8, "dma").help("start-up delay remaining before the copy"),
        // Serial shift engine.
        FieldDef::boundary("serial_bits_remaining", U8, "serial").help("bits left in the transfer"),
        FieldDef::boundary("serial_clock", Bool, "serial")
            .help("internal serial shift clock level"),
        // Cartridge mapper latches.
        FieldDef::boundary("mbc_type", Str, "cartridge").help("mapper identifier"),
        FieldDef::boundary("rom_bank", U16, "cartridge").help("selected ROM bank"),
        FieldDef::boundary("ram_bank", U8, "cartridge").help("selected RAM bank"),
        FieldDef::boundary("ram_enabled", Bool, "cartridge").help("cartridge RAM is enabled"),
        FieldDef::boundary("mbc_mode", U8, "cartridge").help("mapper-specific mode latch"),
        // MBC3 clock-vs-RAM select and real-time clock (present only on an MBC3
        // save, and the RTC fields only when the cart carries a clock).
        FieldDef::boundary("mbc3_clock_sel", U8, "cartridge")
            .help("MBC3 $A000 maps this clock register (0=S 1=M 2=H 3=DL 4=DH); absent ⇒ RAM")
            .nullable(),
        FieldDef::boundary("rtc_seconds", U8, "rtc")
            .help("RTC seconds ($08)")
            .nullable(),
        FieldDef::boundary("rtc_minutes", U8, "rtc")
            .help("RTC minutes ($09)")
            .nullable(),
        FieldDef::boundary("rtc_hours", U8, "rtc")
            .help("RTC hours ($0A)")
            .nullable(),
        FieldDef::boundary("rtc_day_lower", U8, "rtc")
            .help("RTC day low byte ($0B)")
            .nullable(),
        FieldDef::boundary("rtc_day_upper", U8, "rtc")
            .help("RTCDH ($0C) — day bit 8, halt (bit 6), day-carry (bit 7)")
            .nullable(),
        FieldDef::boundary("rtc_latched_seconds", U8, "rtc")
            .help("latched RTC seconds")
            .nullable(),
        FieldDef::boundary("rtc_latched_minutes", U8, "rtc")
            .help("latched RTC minutes")
            .nullable(),
        FieldDef::boundary("rtc_latched_hours", U8, "rtc")
            .help("latched RTC hours")
            .nullable(),
        FieldDef::boundary("rtc_latched_day_lower", U8, "rtc")
            .help("latched RTC day low byte")
            .nullable(),
        FieldDef::boundary("rtc_latched_day_upper", U8, "rtc")
            .help("latched RTCDH")
            .nullable(),
        FieldDef::boundary("rtc_latch_ready", Bool, "rtc")
            .help("a $6000 latch is armed, awaiting its completing write")
            .nullable(),
        // MBC6's second switchable ROM/RAM half and flash latches.
        FieldDef::boundary("mbc6_rom_bank_b", U8, "cartridge")
            .help("MBC6 $6000-$7FFF ROM/flash bank")
            .nullable(),
        FieldDef::boundary("mbc6_ram_bank_b", U8, "cartridge")
            .help("MBC6 $B000-$BFFF RAM bank")
            .nullable(),
        FieldDef::boundary("mbc6_rom_a_flash", Bool, "cartridge")
            .help("MBC6 A-half maps flash rather than ROM")
            .nullable(),
        FieldDef::boundary("mbc6_rom_b_flash", Bool, "cartridge")
            .help("MBC6 B-half maps flash rather than ROM")
            .nullable(),
        FieldDef::boundary("mbc6_flash_enabled", Bool, "cartridge")
            .help("MBC6 flash read path enabled")
            .nullable(),
        // MBC7's split RAM enables, latched accelerometer, and EEPROM write-enable.
        FieldDef::boundary("mbc7_ram_enabled_1", Bool, "cartridge")
            .help("MBC7 enable latch 1 ($0A to $0000-$1FFF)")
            .nullable(),
        FieldDef::boundary("mbc7_ram_enabled_2", Bool, "cartridge")
            .help("MBC7 enable latch 2 ($40 to $4000-$5FFF)")
            .nullable(),
        FieldDef::boundary("mbc7_accel_x", U16, "cartridge")
            .help("MBC7 latched accelerometer X")
            .nullable(),
        FieldDef::boundary("mbc7_accel_y", U16, "cartridge")
            .help("MBC7 latched accelerometer Y")
            .nullable(),
        FieldDef::boundary("mbc7_write_enabled", Bool, "cartridge")
            .help("MBC7 EEPROM write-enable (EWEN) latch")
            .nullable(),
    ]
}

/// The Game Boy RAM regions a save state carries. ROM comes from the cartridge,
/// so it is not a state span.
pub fn dmg_memory_spans() -> Vec<MemorySpan> {
    vec![
        MemorySpan::addressable("vram", 0x8000, 0x2000).help("video RAM"),
        MemorySpan::addressable("wram", 0xC000, 0x2000).help("work RAM"),
        MemorySpan::addressable("oam", 0xFE00, 0x00A0).help("object attribute memory"),
        MemorySpan::addressable("wave_ram", 0xFF30, 0x0010).help("channel 3 wave RAM"),
        MemorySpan::addressable("hram", 0xFF80, 0x007F).help("high RAM"),
        MemorySpan::addressable("cart_ram", 0xA000, 0x2000)
            .optional()
            .help("external cartridge RAM (full contents when banked)"),
    ]
}

/// The DMG framebuffer: 160×144, 2-bit shade indices.
pub fn dmg_frame() -> FrameSpec {
    FrameSpec {
        width: NATIVE_SIZE.0,
        height: Some(NATIVE_SIZE.1),
        format: PixelFormat::Shade2,
    }
}

static DMG_SCHEMA: LazyLock<SystemStateSchema> = LazyLock::new(|| {
    let mut fields = dmg_observable_fields();
    fields.extend(dmg_boundary_fields());
    SystemStateSchema {
        system: "dmg",
        fields,
        memory: dmg_memory_spans(),
        frame: dmg_frame(),
    }
});

/// The DMG hardware state schema.
pub fn dmg_state_schema() -> &'static SystemStateSchema {
    &DMG_SCHEMA
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_well_formed() {
        assert_eq!(dmg_state_schema().check(), Ok(()));
    }

    #[test]
    fn every_field_has_a_tier_and_a_subsystem() {
        for field in &dmg_state_schema().fields {
            assert!(!field.name.is_empty());
            assert!(!field.subsystem.is_empty());
        }
    }

    /// The schema covers every field `snapshot.rs` captures. Each pair maps a
    /// snapshot struct field to its schema field name (identical unless the
    /// snapshot name was emulator-shaped and got a hardware name). If any
    /// snapshot field could not be hardware-named it would appear as an
    /// exclusion here — the list is empty: `snapshot.rs` is by design the
    /// boundary-complete (Tier-2a) set, so every field maps.
    #[test]
    fn schema_covers_every_snapshot_field() {
        // (snapshot.rs field, schema field). Mirrors `snapshot.rs`'s capture set.
        let mapping: &[(&str, &str)] = &[
            // CpuSnapshot
            ("a", "a"),
            ("f", "f"),
            ("b", "b"),
            ("c", "c"),
            ("d", "d"),
            ("e", "e"),
            ("h", "h"),
            ("l", "l"),
            ("sp", "sp"),
            ("pc", "pc"),
            ("ime", "ime"),
            ("if_", "if_"),
            ("ie", "ie"),
            ("halt_state", "cpu_mode"),
            ("ei_delay", "ime_enable_pending"),
            ("halt_bug", "halt_bug"),
            // PpuSnapshot
            ("lcdc", "lcdc"),
            ("stat", "stat"),
            ("ly", "ly"),
            ("lyc", "lyc"),
            ("scy", "scy"),
            ("scx", "scx"),
            ("wy", "wy"),
            ("wx", "wx"),
            ("bgp", "bgp"),
            ("obp0", "obp0"),
            ("obp1", "obp1"),
            ("dma", "dma"),
            ("dot_position", "lx"),
            ("stat_line_was_high", "stat_line"),
            ("window_line_counter", "window_line_counter"),
            // ApuSnapshot
            ("master_vol", "master_vol"),
            ("sound_pan", "sound_pan"),
            ("sound_on", "sound_on"),
            ("ch1_sweep", "ch1_sweep"),
            ("ch1_duty_len", "ch1_duty_len"),
            ("ch1_vol_env", "ch1_vol_env"),
            ("ch1_freq_lo", "ch1_freq_lo"),
            ("ch1_freq_hi", "ch1_freq_hi"),
            ("ch2_duty_len", "ch2_duty_len"),
            ("ch2_vol_env", "ch2_vol_env"),
            ("ch2_freq_lo", "ch2_freq_lo"),
            ("ch2_freq_hi", "ch2_freq_hi"),
            ("ch3_dac", "ch3_dac"),
            ("ch3_len", "ch3_len"),
            ("ch3_vol", "ch3_vol"),
            ("ch3_freq_lo", "ch3_freq_lo"),
            ("ch3_freq_hi", "ch3_freq_hi"),
            ("ch4_len", "ch4_len"),
            ("ch4_vol_env", "ch4_vol_env"),
            ("ch4_freq", "ch4_freq"),
            ("ch4_control", "ch4_control"),
            ("frame_sequencer_step", "frame_sequencer_step"),
            ("prev_div_apu_bit", "prev_div_apu_bit"),
            ("ch1_period", "ch1_period"),
            ("ch1_envelope_timer", "ch1_envelope_timer"),
            ("ch1_sweep_timer", "ch1_sweep_timer"),
            ("ch1_sweep_enabled", "ch1_sweep_enabled"),
            ("ch1_sweep_negate_used", "ch1_sweep_negate_used"),
            ("ch1_length_enabled", "ch1_length_enabled"),
            ("ch2_period", "ch2_period"),
            ("ch2_envelope_timer", "ch2_envelope_timer"),
            ("ch2_length_enabled", "ch2_length_enabled"),
            ("ch3_period", "ch3_period"),
            ("ch3_length_enabled", "ch3_length_enabled"),
            ("ch4_envelope_timer", "ch4_envelope_timer"),
            ("ch4_length_enabled", "ch4_length_enabled"),
            // TimerSnapshot
            ("div", "div"),
            ("tima", "tima"),
            ("tma", "tma"),
            ("tac", "tac"),
            ("internal_counter", "internal_counter"),
            ("overflow_pending", "overflow_pending"),
            ("reloading", "reloading"),
            // DmaSnapshot
            ("active", "dma_active"),
            ("source", "dma_source"),
            ("byte_index", "dma_byte_index"),
            ("delay_remaining", "dma_delay"),
            // SerialSnapshot
            ("sb", "sb"),
            ("sc", "sc"),
            ("bits_remaining", "serial_bits_remaining"),
            ("shift_clock", "serial_clock"),
            // MbcSnapshot
            ("mbc_type", "mbc_type"),
            ("rom_bank", "rom_bank"),
            ("ram_bank", "ram_bank"),
            ("ram_enabled", "ram_enabled"),
            ("mode", "mbc_mode"),
        ];

        let schema = dmg_state_schema();
        let missing: Vec<_> = mapping
            .iter()
            .filter(|(_, schema_name)| schema.field(schema_name).is_none())
            .collect();
        assert!(missing.is_empty(), "schema is missing fields: {missing:?}");
    }

    /// The RAM regions `snapshot.rs::capture_memory` writes each have a span.
    #[test]
    fn schema_covers_every_snapshot_memory_region() {
        let schema = dmg_state_schema();
        for start in [0x8000u32, 0xC000, 0xFE00, 0xFF80, 0xFF30, 0xA000] {
            assert!(
                schema.memory.iter().any(|s| s.start == Some(start)),
                "no span starting at {start:#06X}"
            );
        }
    }
}
