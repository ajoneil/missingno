use crate::common;

fn run_age_fibonacci_test(rom_file: &str) {
    let rom_path = format!("age-test-roms/{rom_file}");
    let mut gb = common::load_rom(&rom_path);
    // AGE tests use LD B,B (0x40) as exit condition
    let found = common::run_until_breakpoint(&mut gb, 1200);
    assert!(
        found,
        "AGE test {rom_file} timed out without reaching LD B,B"
    );

    let cpu = gb.cpu();
    assert!(
        common::check_mooneye_pass(cpu),
        "AGE test {rom_file} failed. Registers: {}",
        common::format_registers(cpu),
    );
}

fn run_age_screenshot_test(rom_file: &str, reference_file: &str) {
    let rom_path = format!("age-test-roms/{rom_file}");
    let reference_path = format!("age-test-roms/{reference_file}");

    let mut gb = common::load_rom(&rom_path);
    let found = common::run_until_breakpoint(&mut gb, 1200);
    assert!(
        found,
        "AGE test {rom_file} timed out without reaching LD B,B"
    );

    let actual = common::screen_to_greyscale(gb.screen());
    let expected = common::load_reference_png(&reference_path);

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
        "AGE test {rom_file}: {mismatches} pixel mismatches"
    );
}

// halt/ — HALT instruction behavior
#[test]
fn ei_halt() {
    run_age_fibonacci_test("ei-halt-dmgC-cgbBCE.gb");
}

#[test]
fn halt_m0_interrupt() {
    run_age_fibonacci_test("halt-m0-interrupt-dmgC-cgbBCE.gb");
}

#[test]
fn halt_prefetch() {
    run_age_fibonacci_test("halt-prefetch-dmgC-cgbBCE.gb");
}

// ly/ — LY register timing
#[test]
fn ly() {
    run_age_fibonacci_test("ly-dmgC-cgbBC.gb");
}

// oam/ — OAM access timing
#[test]
fn oam_read() {
    run_age_fibonacci_test("oam-read-dmgC-cgbBC.gb");
}

#[test]
fn oam_write() {
    run_age_fibonacci_test("oam-write-dmgC.gb");
}

// stat-interrupt/ — STAT interrupt timing
#[test]
fn stat_int() {
    run_age_fibonacci_test("stat-int-dmgC-cgbBCE.gb");
}

// stat-mode/ — STAT mode transitions
#[test]
fn stat_mode() {
    run_age_fibonacci_test("stat-mode-dmgC-cgbBC.gb");
}

// stat-mode-sprites/ — STAT mode with sprites
#[test]
fn stat_mode_sprites() {
    run_age_fibonacci_test("stat-mode-sprites-dmgC-cgbBCE.gb");
}

// stat-mode-window/ — STAT mode with window
#[test]
fn stat_mode_window() {
    run_age_fibonacci_test("stat-mode-window-dmgC.gb");
}

// vram/ — VRAM access timing
#[test]
fn vram_read() {
    run_age_fibonacci_test("vram-read-dmgC.gb");
}

// m3-bg-bgp/ — BGP register changes during mode 3 (screenshot)
#[test]
fn m3_bg_bgp() {
    run_age_screenshot_test("m3-bg-bgp.gb", "m3-bg-bgp-dmgC.png");
}

// m3-bg-lcdc/ — LCDC changes during mode 3 (screenshot, DMG-only ROM)
#[test]
fn m3_bg_lcdc() {
    run_age_screenshot_test("m3-bg-lcdc-nocgb.gb", "m3-bg-lcdc-dmgC.png");
}

// m3-bg-scx/ — SCX changes during mode 3 (screenshot, DMG-only ROM)
#[test]
fn m3_bg_scx() {
    run_age_screenshot_test("m3-bg-scx-nocgb.gb", "m3-bg-scx-dmgC.png");
}
