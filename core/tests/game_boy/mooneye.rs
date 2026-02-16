use crate::common;

const TIMEOUT_FRAMES: u32 = 7200; // 120 seconds at ~60fps

fn run_mooneye_screen_test(rom_path: &str, reference_path: &str) {
    run_mooneye_screen_test_with_timeout(rom_path, reference_path, TIMEOUT_FRAMES);
}

fn run_mooneye_screen_test_with_timeout(rom_path: &str, reference_path: &str, timeout_frames: u32) {
    let mut gb = common::load_rom(rom_path);
    let found_loop = common::run_until_infinite_loop(&mut gb, timeout_frames);
    assert!(
        found_loop,
        "Mooneye test {rom_path} timed out without reaching infinite loop"
    );

    let actual = common::screen_to_greyscale(gb.screen());
    let expected = common::load_reference_png(reference_path);

    let mut mismatches = 0;
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        if a != e {
            if mismatches < 10 {
                let (x, y) = (i % 160, i / 160);
                eprintln!("Pixel mismatch at ({x}, {y}): got 0x{a:02X}, expected 0x{e:02X}");
            }
            mismatches += 1;
        }
    }

    assert_eq!(
        mismatches, 0,
        "Mooneye test {rom_path}: {mismatches} pixel mismatches vs {reference_path}"
    );
}

fn run_mooneye_test(rom_path: &str) {
    let mut gb = common::load_rom(rom_path);
    let mut serial_output = String::new();
    let found_loop = common::run_until_infinite_loop(&mut gb, TIMEOUT_FRAMES);
    // Drain any serial output for diagnostic purposes
    let bytes = gb.drain_serial_output();
    if !bytes.is_empty() {
        serial_output.push_str(&String::from_utf8_lossy(&bytes));
    }
    assert!(
        found_loop,
        "Mooneye test {rom_path} timed out without reaching infinite loop"
    );
    let cpu = gb.cpu();
    if !common::check_mooneye_pass(cpu) {
        // Most Mooneye tests set registers to Fibonacci values (3,5,8,13,21,34)
        // in order as sub-tests pass. Some tests (e.g. lcdon_timing-GS) use
        // quit_inline which sets ALL registers to 0x42 on any failure —
        // detect this pattern to avoid misleading "sub-test 1 failed" reports.
        let all_same =
            cpu.b == cpu.c && cpu.c == cpu.d && cpu.d == cpu.e && cpu.e == cpu.h && cpu.h == cpu.l;
        if all_same && cpu.b != 0 {
            panic!(
                "Mooneye test {rom_path} failed (all registers = 0x{:02X}, ROM uses \
                 uniform failure — sub-test number unknown). Serial: {:?}",
                cpu.b, serial_output,
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
        eprintln!(
            "Sub-tests passed: {passed}/6 (failed at register {failed_reg}, got 0x{failed_val:02X})"
        );
        panic!(
            "Mooneye test {rom_path} failed at sub-test {} (register {failed_reg}=0x{failed_val:02X}, expected {}). \
             Registers: {} Serial: {:?}",
            passed + 1,
            if passed < 6 { fib[passed].1 } else { 0 },
            common::format_registers(cpu),
            serial_output,
        );
    }
}

macro_rules! mooneye_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            run_mooneye_test($path);
        }
    };
}

// acceptance/ — root level
mooneye_test!(add_sp_e_timing, "mooneye/acceptance/add_sp_e_timing.gb");
mooneye_test!(
    boot_div_dmg_abc_mgb,
    "mooneye/acceptance/boot_div-dmgABCmgb.gb"
);
mooneye_test!(
    boot_hwio_dmg_abc_mgb,
    "mooneye/acceptance/boot_hwio-dmgABCmgb.gb"
);
mooneye_test!(boot_regs_dmg_abc, "mooneye/acceptance/boot_regs-dmgABC.gb");
mooneye_test!(call_cc_timing, "mooneye/acceptance/call_cc_timing.gb");
mooneye_test!(call_cc_timing2, "mooneye/acceptance/call_cc_timing2.gb");
mooneye_test!(call_timing, "mooneye/acceptance/call_timing.gb");
mooneye_test!(call_timing2, "mooneye/acceptance/call_timing2.gb");
mooneye_test!(di_timing_gs, "mooneye/acceptance/di_timing-GS.gb");
mooneye_test!(div_timing, "mooneye/acceptance/div_timing.gb");
mooneye_test!(ei_sequence, "mooneye/acceptance/ei_sequence.gb");
mooneye_test!(ei_timing, "mooneye/acceptance/ei_timing.gb");
mooneye_test!(halt_ime0_ei, "mooneye/acceptance/halt_ime0_ei.gb");
mooneye_test!(
    halt_ime0_nointr_timing,
    "mooneye/acceptance/halt_ime0_nointr_timing.gb"
);
mooneye_test!(halt_ime1_timing, "mooneye/acceptance/halt_ime1_timing.gb");
mooneye_test!(
    halt_ime1_timing2_gs,
    "mooneye/acceptance/halt_ime1_timing2-GS.gb"
);
mooneye_test!(if_ie_registers, "mooneye/acceptance/if_ie_registers.gb");
mooneye_test!(intr_timing, "mooneye/acceptance/intr_timing.gb");
mooneye_test!(jp_cc_timing, "mooneye/acceptance/jp_cc_timing.gb");
mooneye_test!(jp_timing, "mooneye/acceptance/jp_timing.gb");
mooneye_test!(ld_hl_sp_e_timing, "mooneye/acceptance/ld_hl_sp_e_timing.gb");
mooneye_test!(oam_dma_restart, "mooneye/acceptance/oam_dma_restart.gb");
mooneye_test!(oam_dma_start, "mooneye/acceptance/oam_dma_start.gb");
mooneye_test!(oam_dma_timing, "mooneye/acceptance/oam_dma_timing.gb");
mooneye_test!(pop_timing, "mooneye/acceptance/pop_timing.gb");
mooneye_test!(push_timing, "mooneye/acceptance/push_timing.gb");
mooneye_test!(rapid_di_ei, "mooneye/acceptance/rapid_di_ei.gb");
mooneye_test!(ret_cc_timing, "mooneye/acceptance/ret_cc_timing.gb");
mooneye_test!(ret_timing, "mooneye/acceptance/ret_timing.gb");
mooneye_test!(reti_intr_timing, "mooneye/acceptance/reti_intr_timing.gb");
mooneye_test!(reti_timing, "mooneye/acceptance/reti_timing.gb");
mooneye_test!(rst_timing, "mooneye/acceptance/rst_timing.gb");

// acceptance/bits/
mooneye_test!(bits_mem_oam, "mooneye/acceptance/bits/mem_oam.gb");
mooneye_test!(bits_reg_f, "mooneye/acceptance/bits/reg_f.gb");
mooneye_test!(
    bits_unused_hwio_gs,
    "mooneye/acceptance/bits/unused_hwio-GS.gb"
);

// acceptance/instr/
mooneye_test!(instr_daa, "mooneye/acceptance/instr/daa.gb");

// acceptance/interrupts/
mooneye_test!(
    interrupts_ie_push,
    "mooneye/acceptance/interrupts/ie_push.gb"
);

// acceptance/oam_dma/
mooneye_test!(oam_dma_basic, "mooneye/acceptance/oam_dma/basic.gb");
mooneye_test!(oam_dma_reg_read, "mooneye/acceptance/oam_dma/reg_read.gb");
mooneye_test!(
    oam_dma_sources_gs,
    "mooneye/acceptance/oam_dma/sources-GS.gb"
);

// acceptance/ppu/
mooneye_test!(
    ppu_hblank_ly_scx_timing_gs,
    "mooneye/acceptance/ppu/hblank_ly_scx_timing-GS.gb"
);
mooneye_test!(
    ppu_intr_1_2_timing_gs,
    "mooneye/acceptance/ppu/intr_1_2_timing-GS.gb"
);
mooneye_test!(
    ppu_intr_2_0_timing,
    "mooneye/acceptance/ppu/intr_2_0_timing.gb"
);
mooneye_test!(
    ppu_intr_2_mode0_timing,
    "mooneye/acceptance/ppu/intr_2_mode0_timing.gb"
);
mooneye_test!(
    ppu_intr_2_mode0_timing_sprites,
    "mooneye/acceptance/ppu/intr_2_mode0_timing_sprites.gb"
);
mooneye_test!(
    ppu_intr_2_mode3_timing,
    "mooneye/acceptance/ppu/intr_2_mode3_timing.gb"
);
mooneye_test!(
    ppu_intr_2_oam_ok_timing,
    "mooneye/acceptance/ppu/intr_2_oam_ok_timing.gb"
);
mooneye_test!(
    ppu_lcdon_timing_gs,
    "mooneye/acceptance/ppu/lcdon_timing-GS.gb"
);
mooneye_test!(
    ppu_lcdon_write_timing_gs,
    "mooneye/acceptance/ppu/lcdon_write_timing-GS.gb"
);
mooneye_test!(
    ppu_stat_irq_blocking,
    "mooneye/acceptance/ppu/stat_irq_blocking.gb"
);
mooneye_test!(
    ppu_stat_lyc_onoff,
    "mooneye/acceptance/ppu/stat_lyc_onoff.gb"
);

mooneye_test!(
    ppu_vblank_stat_intr_gs,
    "mooneye/acceptance/ppu/vblank_stat_intr-GS.gb"
);

// acceptance/serial/
mooneye_test!(
    serial_boot_sclk_align_dmg,
    "mooneye/acceptance/serial/boot_sclk_align-dmgABCmgb.gb"
);

// acceptance/timer/
mooneye_test!(timer_div_write, "mooneye/acceptance/timer/div_write.gb");
mooneye_test!(
    timer_rapid_toggle,
    "mooneye/acceptance/timer/rapid_toggle.gb"
);
mooneye_test!(timer_tim00, "mooneye/acceptance/timer/tim00.gb");
mooneye_test!(
    timer_tim00_div_trigger,
    "mooneye/acceptance/timer/tim00_div_trigger.gb"
);
mooneye_test!(timer_tim01, "mooneye/acceptance/timer/tim01.gb");
mooneye_test!(
    timer_tim01_div_trigger,
    "mooneye/acceptance/timer/tim01_div_trigger.gb"
);
mooneye_test!(timer_tim10, "mooneye/acceptance/timer/tim10.gb");
mooneye_test!(
    timer_tim10_div_trigger,
    "mooneye/acceptance/timer/tim10_div_trigger.gb"
);
mooneye_test!(timer_tim11, "mooneye/acceptance/timer/tim11.gb");
mooneye_test!(
    timer_tim11_div_trigger,
    "mooneye/acceptance/timer/tim11_div_trigger.gb"
);
mooneye_test!(timer_tima_reload, "mooneye/acceptance/timer/tima_reload.gb");
mooneye_test!(
    timer_tima_write_reloading,
    "mooneye/acceptance/timer/tima_write_reloading.gb"
);
mooneye_test!(
    timer_tma_write_reloading,
    "mooneye/acceptance/timer/tma_write_reloading.gb"
);

// madness/ — obscure hardware edge cases (screenshot comparison)
//
// mgb_oam_dma_halt_sprites: Disabled — this test is MGB (Game Boy Pocket)
// specific. The expected PNG matches MGB behavior only; DMG produces a
// different (undocumented) result due to model-specific OAM corruption
// patterns during halted DMA. No DMG reference image exists upstream or
// elsewhere. See receipts/research/systems/game-boy/testing/
// madness-mgb-oam-dma-halt-sprites.md for details.
//
// #[test]
// fn madness_mgb_oam_dma_halt_sprites() {
//     run_mooneye_screen_test_with_timeout(
//         "mooneye/madness/mgb_oam_dma_halt_sprites.gb",
//         "mooneye/madness/mgb_oam_dma_halt_sprites_expected.png",
//         600,
//     );
// }

// manual-only/ — visual tests requiring screenshot comparison
#[test]
fn manual_sprite_priority() {
    run_mooneye_screen_test(
        "mooneye/manual-only/sprite_priority.gb",
        "mooneye/manual-only/sprite_priority-dmg.png",
    );
}

// emulator-only/mbc1/ — MBC1 cartridge controller tests
mooneye_test!(mbc1_bits_bank1, "mooneye/emulator-only/mbc1/bits_bank1.gb");
mooneye_test!(mbc1_bits_bank2, "mooneye/emulator-only/mbc1/bits_bank2.gb");
mooneye_test!(mbc1_bits_mode, "mooneye/emulator-only/mbc1/bits_mode.gb");
mooneye_test!(mbc1_bits_ramg, "mooneye/emulator-only/mbc1/bits_ramg.gb");
mooneye_test!(
    mbc1_multicart_rom_8mb,
    "mooneye/emulator-only/mbc1/multicart_rom_8Mb.gb"
);
mooneye_test!(mbc1_ram_64kb, "mooneye/emulator-only/mbc1/ram_64kb.gb");
mooneye_test!(mbc1_ram_256kb, "mooneye/emulator-only/mbc1/ram_256kb.gb");
mooneye_test!(mbc1_rom_512kb, "mooneye/emulator-only/mbc1/rom_512kb.gb");
mooneye_test!(mbc1_rom_1mb, "mooneye/emulator-only/mbc1/rom_1Mb.gb");
mooneye_test!(mbc1_rom_2mb, "mooneye/emulator-only/mbc1/rom_2Mb.gb");
mooneye_test!(mbc1_rom_4mb, "mooneye/emulator-only/mbc1/rom_4Mb.gb");
mooneye_test!(mbc1_rom_8mb, "mooneye/emulator-only/mbc1/rom_8Mb.gb");
mooneye_test!(mbc1_rom_16mb, "mooneye/emulator-only/mbc1/rom_16Mb.gb");

// emulator-only/mbc2/ — MBC2 cartridge controller tests
mooneye_test!(mbc2_bits_ramg, "mooneye/emulator-only/mbc2/bits_ramg.gb");
mooneye_test!(mbc2_bits_romb, "mooneye/emulator-only/mbc2/bits_romb.gb");
mooneye_test!(
    mbc2_bits_unused,
    "mooneye/emulator-only/mbc2/bits_unused.gb"
);
mooneye_test!(mbc2_ram, "mooneye/emulator-only/mbc2/ram.gb");
mooneye_test!(mbc2_rom_512kb, "mooneye/emulator-only/mbc2/rom_512kb.gb");
mooneye_test!(mbc2_rom_1mb, "mooneye/emulator-only/mbc2/rom_1Mb.gb");
mooneye_test!(mbc2_rom_2mb, "mooneye/emulator-only/mbc2/rom_2Mb.gb");

// emulator-only/mbc5/ — MBC5 cartridge controller tests
mooneye_test!(mbc5_rom_512kb, "mooneye/emulator-only/mbc5/rom_512kb.gb");
mooneye_test!(mbc5_rom_1mb, "mooneye/emulator-only/mbc5/rom_1Mb.gb");
mooneye_test!(mbc5_rom_2mb, "mooneye/emulator-only/mbc5/rom_2Mb.gb");
mooneye_test!(mbc5_rom_4mb, "mooneye/emulator-only/mbc5/rom_4Mb.gb");
mooneye_test!(mbc5_rom_8mb, "mooneye/emulator-only/mbc5/rom_8Mb.gb");
mooneye_test!(mbc5_rom_16mb, "mooneye/emulator-only/mbc5/rom_16Mb.gb");
mooneye_test!(mbc5_rom_32mb, "mooneye/emulator-only/mbc5/rom_32Mb.gb");
mooneye_test!(mbc5_rom_64mb, "mooneye/emulator-only/mbc5/rom_64Mb.gb");
