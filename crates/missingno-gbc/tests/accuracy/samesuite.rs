//! SameSuite — CGB-C-compatible subset.
//!
//! Filename suffix convention (SameBoy):
//! - no suffix → works on every CGB revision (and often DMG)
//! - `-cgb0BC` → CGB-0, B, C (✓ includes C)
//! - `-cgb0B`, `-cgb0`, `-cgbB`, `-A`, `-cgbDE`, `-cgbE` → revisions without C (excluded)
//! - SGB tests excluded entirely
//!
//! Omitted by *content*, not by suffix (these ROMs are suffix-less, so the
//! rule above doesn't catch them): the NRx2 "Zombie Mode" glitch ROMs
//! `channel_{1,2}_nrx2_glitch` and `channel_{1,2}_restart_nrx2_glitch`.
//! SameSuite measures that glitch's volume arithmetic only on revision E,
//! so their expected values are CGB-E-only and a CGB-C core cannot satisfy
//! them. NOTE: `channel_{1,2}_nrx2_speed_change` is a *different*,
//! revision-shared test (envelope-enable timing) and IS kept — don't
//! confuse the two.
//!
//! The channel 1/2/4 `CorrectResults` are themselves measured on CGB-E; on
//! real CGB-C those tests fail via the PCM12/PCM34 same-M-cycle read glitch
//! (rev C and older), which we don't model — so we pass them with clean
//! (E-like) PCM reads, and the remaining failures are ordinary shared APU
//! timing bugs, not CGB-E behaviour.
//!
//! Three ROMs that work on DMG too (`channel_3_wave_ram_*`,
//! `ei_delay_halt`) live in `missingno-gb`'s roms tree and use
//! [`common::load_rom`]; the remainder live in this crate's roms tree
//! and use [`common::load_cgb_rom`].
//!
//! Many of these will fail until CGB APU quirks, double-speed mode,
//! HDMA/GDMA, and CGB palette memory land.

use crate::common;
use missingno_gbc::GameBoyColor;

// Many samesuite ROMs don't enable the LCD, so frame-based runners
// hang. We use an instruction-budget runner instead. 2_000_000
// instructions = roughly 30 LCD frames at single speed; plenty for
// the passing tests, bounded enough to keep failures fast.
const MAX_INSTRUCTIONS: u32 = 2_000_000;

fn check_pass(gbc: &GameBoyColor, rom_path: &str) {
    let cpu = gbc.cpu();
    if common::check_mooneye_pass(cpu) {
        return;
    }

    let all_same =
        cpu.b == cpu.c && cpu.c == cpu.d && cpu.d == cpu.e && cpu.e == cpu.h && cpu.h == cpu.l;
    if all_same && cpu.b != 0 {
        panic!(
            "SameSuite test {rom_path} failed (all registers = 0x{:02X})",
            cpu.b
        );
    }

    let fib = [
        (cpu.b, 3, "B"),
        (cpu.c, 5, "C"),
        (cpu.d, 8, "D"),
        (cpu.e, 13, "E"),
        (cpu.h, 21, "H"),
        (cpu.l, 34, "L"),
    ];
    let passed = fib
        .iter()
        .take_while(|(val, expected, _)| val == expected)
        .count();
    let failed_reg = if passed < 6 { fib[passed].2 } else { "?" };
    let failed_val = if passed < 6 { fib[passed].0 } else { 0 };
    panic!(
        "SameSuite test {rom_path} failed at sub-test {} (register {failed_reg}=0x{failed_val:02X}). \
         Registers: {}",
        passed + 1,
        common::format_registers(cpu),
    );
}

fn run_shared(rom_path: &str) {
    let mut gbc = common::load_rom(rom_path);
    let found_loop = common::run_until_infinite_loop_no_lcd(&mut gbc, MAX_INSTRUCTIONS);
    assert!(
        found_loop,
        "SameSuite test {rom_path} timed out without reaching infinite loop"
    );
    check_pass(&gbc, rom_path);
}

fn run_cgb(rom_path: &str) {
    let mut gbc = common::load_cgb_rom(rom_path);
    let found_loop = common::run_until_infinite_loop_no_lcd(&mut gbc, MAX_INSTRUCTIONS);
    assert!(
        found_loop,
        "SameSuite test {rom_path} timed out without reaching infinite loop"
    );
    check_pass(&gbc, rom_path);
}

macro_rules! shared {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            run_shared($path);
        }
    };
}

macro_rules! cgb {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            run_cgb($path);
        }
    };
}

// apu/channel_3 / interrupt — DMG-compatible, shared with missingno-gb.
shared!(
    channel_3_wave_ram_dac_on_rw,
    "samesuite/apu/channel_3/channel_3_wave_ram_dac_on_rw.gb"
);
shared!(
    channel_3_wave_ram_locked_write,
    "samesuite/apu/channel_3/channel_3_wave_ram_locked_write.gb"
);
shared!(ei_delay_halt, "samesuite/interrupt/ei_delay_halt.gb");

// apu/channel_1
cgb!(
    channel_1_align,
    "samesuite/apu/channel_1/channel_1_align.gb"
);
cgb!(
    channel_1_align_cpu,
    "samesuite/apu/channel_1/channel_1_align_cpu.gb"
);
cgb!(
    channel_1_delay,
    "samesuite/apu/channel_1/channel_1_delay.gb"
);
cgb!(channel_1_duty, "samesuite/apu/channel_1/channel_1_duty.gb");
cgb!(
    channel_1_duty_delay,
    "samesuite/apu/channel_1/channel_1_duty_delay.gb"
);
cgb!(
    channel_1_freq_change,
    "samesuite/apu/channel_1/channel_1_freq_change.gb"
);
cgb!(
    channel_1_freq_change_timing_cgb0bc,
    "samesuite/apu/channel_1/channel_1_freq_change_timing-cgb0BC.gb"
);
cgb!(
    channel_1_nrx2_speed_change,
    "samesuite/apu/channel_1/channel_1_nrx2_speed_change.gb"
);
cgb!(
    channel_1_restart,
    "samesuite/apu/channel_1/channel_1_restart.gb"
);
cgb!(
    channel_1_stop_div,
    "samesuite/apu/channel_1/channel_1_stop_div.gb"
);
cgb!(
    channel_1_stop_restart,
    "samesuite/apu/channel_1/channel_1_stop_restart.gb"
);
cgb!(
    channel_1_sweep,
    "samesuite/apu/channel_1/channel_1_sweep.gb"
);
cgb!(
    channel_1_sweep_restart,
    "samesuite/apu/channel_1/channel_1_sweep_restart.gb"
);
cgb!(
    channel_1_sweep_restart_2,
    "samesuite/apu/channel_1/channel_1_sweep_restart_2.gb"
);
cgb!(
    channel_1_volume,
    "samesuite/apu/channel_1/channel_1_volume.gb"
);
cgb!(
    channel_1_volume_div,
    "samesuite/apu/channel_1/channel_1_volume_div.gb"
);

// apu/channel_2
cgb!(
    channel_2_align,
    "samesuite/apu/channel_2/channel_2_align.gb"
);
cgb!(
    channel_2_align_cpu,
    "samesuite/apu/channel_2/channel_2_align_cpu.gb"
);
cgb!(
    channel_2_delay,
    "samesuite/apu/channel_2/channel_2_delay.gb"
);
cgb!(channel_2_duty, "samesuite/apu/channel_2/channel_2_duty.gb");
cgb!(
    channel_2_duty_delay,
    "samesuite/apu/channel_2/channel_2_duty_delay.gb"
);
cgb!(
    channel_2_freq_change,
    "samesuite/apu/channel_2/channel_2_freq_change.gb"
);
cgb!(
    channel_2_nrx2_speed_change,
    "samesuite/apu/channel_2/channel_2_nrx2_speed_change.gb"
);
cgb!(
    channel_2_restart,
    "samesuite/apu/channel_2/channel_2_restart.gb"
);
cgb!(
    channel_2_stop_div,
    "samesuite/apu/channel_2/channel_2_stop_div.gb"
);
cgb!(
    channel_2_stop_restart,
    "samesuite/apu/channel_2/channel_2_stop_restart.gb"
);
cgb!(
    channel_2_volume,
    "samesuite/apu/channel_2/channel_2_volume.gb"
);
cgb!(
    channel_2_volume_div,
    "samesuite/apu/channel_2/channel_2_volume_div.gb"
);

// apu/channel_3 (CGB-only variants — channel_3_wave_ram_* are shared above)
cgb!(
    channel_3_and_glitch,
    "samesuite/apu/channel_3/channel_3_and_glitch.gb"
);
cgb!(
    channel_3_delay,
    "samesuite/apu/channel_3/channel_3_delay.gb"
);
cgb!(
    channel_3_first_sample,
    "samesuite/apu/channel_3/channel_3_first_sample.gb"
);
cgb!(
    channel_3_freq_change_delay,
    "samesuite/apu/channel_3/channel_3_freq_change_delay.gb"
);
cgb!(
    channel_3_restart_delay,
    "samesuite/apu/channel_3/channel_3_restart_delay.gb"
);
cgb!(
    channel_3_restart_during_delay,
    "samesuite/apu/channel_3/channel_3_restart_during_delay.gb"
);
cgb!(
    channel_3_restart_stop_delay,
    "samesuite/apu/channel_3/channel_3_restart_stop_delay.gb"
);
cgb!(
    channel_3_shift_delay,
    "samesuite/apu/channel_3/channel_3_shift_delay.gb"
);
cgb!(
    channel_3_shift_skip_delay,
    "samesuite/apu/channel_3/channel_3_shift_skip_delay.gb"
);
cgb!(
    channel_3_stop_delay,
    "samesuite/apu/channel_3/channel_3_stop_delay.gb"
);
cgb!(
    channel_3_stop_div,
    "samesuite/apu/channel_3/channel_3_stop_div.gb"
);
cgb!(
    channel_3_wave_ram_sync,
    "samesuite/apu/channel_3/channel_3_wave_ram_sync.gb"
);

// apu/channel_4
cgb!(
    channel_4_align,
    "samesuite/apu/channel_4/channel_4_align.gb"
);
cgb!(
    channel_4_delay,
    "samesuite/apu/channel_4/channel_4_delay.gb"
);
cgb!(
    channel_4_equivalent_frequencies,
    "samesuite/apu/channel_4/channel_4_equivalent_frequencies.gb"
);
cgb!(
    channel_4_freq_change,
    "samesuite/apu/channel_4/channel_4_freq_change.gb"
);
cgb!(
    channel_4_frequency_alignment,
    "samesuite/apu/channel_4/channel_4_frequency_alignment.gb"
);
cgb!(channel_4_lfsr, "samesuite/apu/channel_4/channel_4_lfsr.gb");
cgb!(
    channel_4_lfsr15,
    "samesuite/apu/channel_4/channel_4_lfsr15.gb"
);
cgb!(
    channel_4_lfsr_15_7,
    "samesuite/apu/channel_4/channel_4_lfsr_15_7.gb"
);
cgb!(
    channel_4_lfsr_7_15,
    "samesuite/apu/channel_4/channel_4_lfsr_7_15.gb"
);
cgb!(
    channel_4_lfsr_restart,
    "samesuite/apu/channel_4/channel_4_lfsr_restart.gb"
);
cgb!(
    channel_4_lfsr_restart_fast,
    "samesuite/apu/channel_4/channel_4_lfsr_restart_fast.gb"
);
cgb!(
    channel_4_volume_div,
    "samesuite/apu/channel_4/channel_4_volume_div.gb"
);

// apu/ — div tests
cgb!(div_write_trigger, "samesuite/apu/div_write_trigger.gb");
cgb!(
    div_write_trigger_10,
    "samesuite/apu/div_write_trigger_10.gb"
);
cgb!(
    div_write_trigger_volume,
    "samesuite/apu/div_write_trigger_volume.gb"
);
cgb!(
    div_write_trigger_volume_10,
    "samesuite/apu/div_write_trigger_volume_10.gb"
);
cgb!(
    div_trigger_volume_10,
    "samesuite/apu/div_trigger_volume_10.gb"
);

// dma — CGB-only DMA (GDMA / HDMA)
cgb!(gbc_dma_cont, "samesuite/dma/gbc_dma_cont.gb");
cgb!(gdma_addr_mask, "samesuite/dma/gdma_addr_mask.gb");
cgb!(hdma_lcd_off, "samesuite/dma/hdma_lcd_off.gb");
cgb!(hdma_mode0, "samesuite/dma/hdma_mode0.gb");

// ppu — CGB-only ($FF68 BGPI)
cgb!(
    blocking_bgpi_increase,
    "samesuite/ppu/blocking_bgpi_increase.gb"
);
