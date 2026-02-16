use crate::common;

fn run_blargg_test(rom_path: &str) {
    run_blargg_test_with_timeout(rom_path, 3600);
}

fn run_blargg_test_with_timeout(rom_path: &str, timeout_frames: u32) {
    let mut gb = common::load_rom(rom_path);
    let output = common::run_until_serial_match(&mut gb, &["Passed", "Failed"], timeout_frames);
    assert!(
        output.contains("Passed"),
        "Blargg test {rom_path} failed. Serial output:\n{output}"
    );
}

fn run_blargg_screen_test(rom_path: &str, reference_path: &str, timeout_frames: u32) {
    let mut gb = common::load_rom(rom_path);
    let found_loop = common::run_until_infinite_loop(&mut gb, timeout_frames);
    assert!(
        found_loop,
        "Blargg test {rom_path} timed out without reaching infinite loop"
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
        "Blargg test {rom_path}: {mismatches} pixel mismatches vs {reference_path}"
    );
}

#[test]
fn cpu_instrs() {
    run_blargg_test_with_timeout("blargg/cpu_instrs/cpu_instrs.gb", 7200);
}

// cpu_instrs — individual sub-tests (faster to isolate failures)
#[test]
fn cpu_instrs_01_special() {
    run_blargg_test("blargg/cpu_instrs/individual/01-special.gb");
}

#[test]
fn cpu_instrs_02_interrupts() {
    run_blargg_test("blargg/cpu_instrs/individual/02-interrupts.gb");
}

#[test]
fn cpu_instrs_03_op_sp_hl() {
    run_blargg_test("blargg/cpu_instrs/individual/03-op sp,hl.gb");
}

#[test]
fn cpu_instrs_04_op_r_imm() {
    run_blargg_test("blargg/cpu_instrs/individual/04-op r,imm.gb");
}

#[test]
fn cpu_instrs_05_op_rp() {
    run_blargg_test("blargg/cpu_instrs/individual/05-op rp.gb");
}

#[test]
fn cpu_instrs_06_ld_r_r() {
    run_blargg_test("blargg/cpu_instrs/individual/06-ld r,r.gb");
}

#[test]
fn cpu_instrs_07_jr_jp_call_ret_rst() {
    run_blargg_test("blargg/cpu_instrs/individual/07-jr,jp,call,ret,rst.gb");
}

#[test]
fn cpu_instrs_08_misc_instrs() {
    run_blargg_test("blargg/cpu_instrs/individual/08-misc instrs.gb");
}

#[test]
fn cpu_instrs_09_op_r_r() {
    run_blargg_test("blargg/cpu_instrs/individual/09-op r,r.gb");
}

#[test]
fn cpu_instrs_10_bit_ops() {
    run_blargg_test("blargg/cpu_instrs/individual/10-bit ops.gb");
}

#[test]
fn cpu_instrs_11_op_a_hl() {
    run_blargg_test("blargg/cpu_instrs/individual/11-op a,(hl).gb");
}

// Instruction timing
#[test]
fn instr_timing() {
    run_blargg_test("blargg/instr_timing/instr_timing.gb");
}

// Memory timing
#[test]
fn mem_timing() {
    run_blargg_test("blargg/mem_timing/mem_timing.gb");
}

// Memory timing 2 (screen-only, no serial output — uses screenshot comparison)
#[test]
fn mem_timing_2() {
    run_blargg_screen_test(
        "blargg/mem_timing-2/mem_timing.gb",
        "blargg/mem_timing-2/mem_timing-dmg-cgb.png",
        1200,
    );
}

// Halt bug (screen-only, no serial output — uses screenshot comparison)
#[test]
fn halt_bug() {
    run_blargg_screen_test("blargg/halt_bug.gb", "blargg/halt_bug-dmg-cgb.png", 1200);
}

// interrupt_time — removed. This is a CGB-only test (.define REQUIRE_CGB 1)
// that tests interrupt timing at both normal and double CPU speed. On DMG
// hardware the ROM immediately displays "Failed" and halts. Matching that
// failure screenshot tests nothing about emulation accuracy.
// See receipts/research/systems/game-boy/testing/test-rom-reliability.md

// DMG sound — individual sub-tests (cart RAM result, not serial)
fn run_dmg_sound_single(name: &str, timeout_frames: u32) {
    let rom_path = format!("blargg/dmg_sound/rom_singles/{name}.gb");
    run_blargg_cart_ram_test(&rom_path, timeout_frames);
}

#[test]
fn dmg_sound_01_registers() {
    run_dmg_sound_single("01-registers", 600);
}

#[test]
fn dmg_sound_02_len_ctr() {
    run_dmg_sound_single("02-len ctr", 600);
}

#[test]
fn dmg_sound_03_trigger() {
    // CH3's length counter goes up to 256, so get_len_a takes ~60 frames per
    // measurement. With many sub-tests, this needs a generous timeout.
    run_dmg_sound_single("03-trigger", 3600);
}

#[test]
fn dmg_sound_04_sweep() {
    run_dmg_sound_single("04-sweep", 600);
}

#[test]
fn dmg_sound_05_sweep_details() {
    run_dmg_sound_single("05-sweep details", 600);
}

#[test]
fn dmg_sound_06_overflow_on_trigger() {
    run_dmg_sound_single("06-overflow on trigger", 600);
}

#[test]
fn dmg_sound_07_len_sweep_period_sync() {
    run_dmg_sound_single("07-len sweep period sync", 600);
}

#[test]
fn dmg_sound_08_len_ctr_during_power() {
    run_dmg_sound_single("08-len ctr during power", 600);
}

#[test]
fn dmg_sound_09_wave_read_while_on() {
    run_dmg_sound_single("09-wave read while on", 600);
}

#[test]
fn dmg_sound_10_wave_trigger_while_on() {
    run_dmg_sound_single("10-wave trigger while on", 600);
}

#[test]
fn dmg_sound_11_regs_after_power() {
    run_dmg_sound_single("11-regs after power", 600);
}

#[test]
fn dmg_sound_12_wave_write_while_on() {
    run_dmg_sound_single("12-wave write while on", 600);
}

// DMG sound — combined (screen-only, no serial output — uses screenshot comparison)
#[test]
fn dmg_sound() {
    run_blargg_screen_test(
        "blargg/dmg_sound/dmg_sound.gb",
        "blargg/dmg_sound/dmg_sound-dmg.png",
        3600,
    );
}

// OAM bug — combined (screen-only, no serial output — uses screenshot comparison)
#[test]
fn oam_bug() {
    run_blargg_screen_test(
        "blargg/oam_bug/oam_bug.gb",
        "blargg/oam_bug/oam_bug-dmg.png",
        1200,
    );
}

// OAM bug — individual sub-tests (screen-only, use cart RAM for result text)
fn run_blargg_cart_ram_test(rom_path: &str, timeout_frames: u32) {
    let mut gb = common::load_rom(rom_path);
    let found_loop = common::run_until_infinite_loop(&mut gb, timeout_frames);

    // Read result text from cart RAM at $A004 (zero-terminated string)
    // Signature at $A001-$A003 should be $DE $B0 $61
    let mut text = String::new();
    for i in 0..512u16 {
        let b = gb.read(0xA004 + i);
        if b == 0 {
            break;
        }
        text.push(b as char);
    }

    let result_code = gb.read(0xA000);

    assert!(
        found_loop,
        "Blargg test {rom_path} timed out. Result code: 0x{result_code:02X}. Output:\n{text}"
    );

    assert_eq!(
        result_code, 0,
        "Blargg test {rom_path} failed with code {result_code}. Output:\n{text}"
    );
}

#[test]
fn oam_bug_1_lcd_sync() {
    run_blargg_cart_ram_test("blargg/oam_bug/rom_singles/1-lcd_sync.gb", 1200);
}

#[test]
fn oam_bug_2_causes() {
    run_blargg_cart_ram_test("blargg/oam_bug/rom_singles/2-causes.gb", 1200);
}

#[test]
fn oam_bug_3_non_causes() {
    run_blargg_cart_ram_test("blargg/oam_bug/rom_singles/3-non_causes.gb", 1200);
}

#[test]
fn oam_bug_4_scanline_timing() {
    run_blargg_cart_ram_test("blargg/oam_bug/rom_singles/4-scanline_timing.gb", 1200);
}

#[test]
fn oam_bug_5_timing_bug() {
    run_blargg_cart_ram_test("blargg/oam_bug/rom_singles/5-timing_bug.gb", 1200);
}

#[test]
fn oam_bug_6_timing_no_bug() {
    run_blargg_cart_ram_test("blargg/oam_bug/rom_singles/6-timing_no_bug.gb", 1200);
}

#[test]
#[ignore] // Takes too long: 116 iterations of LCD off/on/compare/print cycles
fn oam_bug_7_timing_effect() {
    run_blargg_cart_ram_test("blargg/oam_bug/rom_singles/7-timing_effect.gb", 6000);
}

#[test]
fn oam_bug_8_instr_effect() {
    run_blargg_cart_ram_test("blargg/oam_bug/rom_singles/8-instr_effect.gb", 1200);
}
