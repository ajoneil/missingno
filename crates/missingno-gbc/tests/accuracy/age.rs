//! AGE test ROMs — CGB-C-compatible subset.
//!
//! AGE ROM filenames carry device tags (e.g. `-cgbBCE`, `-ncmBC`,
//! `-dmgC-cgbBC`). For CGB-C target, we include any ROM whose tag
//! list contains `C`:
//!
//! - `-cgbBC`, `-cgbBCE`, `-cgbBCDE`, `-cgbABCDE` → CGB target
//! - `-ncmBC`, `-ncmBCE` → CGB running a DMG ROM in compat mode
//! - `-dmgC-cgb…BC*` → both DMG and CGB targets
//! - no suffix → all-device
//! - `-ds` → double-speed (CGB only)
//!
//! Excluded: CGB-E-only, CGB-B-only, `-nocgb` (DMG only).
//!
//! Many tests here will fail until CGB-specific features land
//! (palette memory, banking, HDMA, double-speed). The screenshot
//! tests use `-cgbBCE.png` references (or `-ncmBC.png` for ROMs that
//! target the CGB-in-DMG-compat mode).

use crate::common;
use missingno_gbc::GameBoyColor;

fn check_pass(gbc: &GameBoyColor, rom_file: &str) {
    let cpu = gbc.cpu();
    if !common::check_mooneye_pass(cpu) {
        panic!(
            "AGE test {rom_file} failed.\n\
             Registers: {} (B=0 is the fail signal; C/D/E/H/L are residue from the shared \
             \"TEST FAILED!\" display path).\n\
             WRAM dump:{}",
            common::format_registers(cpu),
            common::format_wram_dump(gbc, 0xC000, 0x800),
        );
    }
}

fn run_register_shared(rom_file: &str) {
    let mut gbc = common::load_rom(&format!("age-test-roms/{rom_file}"));
    let found = common::run_until_breakpoint(&mut gbc, 1200);
    assert!(
        found,
        "AGE test {rom_file} timed out without reaching LD B,B"
    );
    check_pass(&gbc, rom_file);
}

fn run_register_cgb(rom_file: &str) {
    let mut gbc = common::load_cgb_rom(&format!("age-test-roms/{rom_file}"));
    let found = common::run_until_breakpoint(&mut gbc, 1200);
    assert!(
        found,
        "AGE test {rom_file} timed out without reaching LD B,B"
    );
    check_pass(&gbc, rom_file);
}

fn run_screen_shared(rom_file: &str, reference_file: &str) {
    let mut gbc = common::load_rom(&format!("age-test-roms/{rom_file}"));
    let found = common::run_until_breakpoint(&mut gbc, 1200);
    assert!(
        found,
        "AGE test {rom_file} timed out without reaching LD B,B"
    );

    let actual = gbc.screen().to_greyscale_bytes();
    let expected = common::load_cgb_reference_png(&format!("age-test-roms/{reference_file}"));

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

fn run_screen_cgb(rom_file: &str, reference_file: &str) {
    let mut gbc = common::load_cgb_rom(&format!("age-test-roms/{rom_file}"));
    let found = common::run_until_breakpoint(&mut gbc, 1200);
    assert!(
        found,
        "AGE test {rom_file} timed out without reaching LD B,B"
    );

    let actual = gbc.screen().to_greyscale_bytes();
    let expected = common::load_cgb_reference_png(&format!("age-test-roms/{reference_file}"));

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

// halt/ — DMG+CGB shared (already in gb crate, ROMs loaded from there)
#[test]
fn ei_halt() {
    run_register_shared("ei-halt-dmgC-cgbBCE.gb");
}

#[test]
fn halt_m0_interrupt() {
    run_register_shared("halt-m0-interrupt-dmgC-cgbBCE.gb");
}

#[test]
fn halt_prefetch() {
    run_register_shared("halt-prefetch-dmgC-cgbBCE.gb");
}

// ly/
#[test]
fn ly() {
    run_register_shared("ly-dmgC-cgbBC.gb");
}

#[test]
fn ly_ncm() {
    run_register_cgb("ly-ncmBC.gb");
}

// lcd-align-ly/ — CGB-only
#[test]
fn lcd_align_ly() {
    run_register_cgb("lcd-align-ly-cgbBC.gb");
}

// oam/
#[test]
fn oam_read() {
    run_register_shared("oam-read-dmgC-cgbBC.gb");
}

#[test]
fn oam_read_ncm() {
    run_register_cgb("oam-read-ncmBC.gb");
}

#[test]
fn oam_write() {
    run_register_cgb("oam-write-cgbBCE.gb");
}

#[test]
fn oam_write_ncm() {
    run_register_cgb("oam-write-ncmBCE.gb");
}

// spsw/ — STOP / double-speed wakeup (CGB-only)
#[test]
fn spsw_ch2_lc_delay() {
    run_register_cgb("spsw-ch2-lc-delay-cgbBCE.gb");
}

#[test]
fn spsw_div() {
    run_register_cgb("spsw-div-cgbBCE.gb");
}

#[test]
fn spsw_interrupts() {
    run_register_cgb("spsw-interrupts-cgbBC.gb");
}

#[test]
fn spsw_mode0() {
    run_register_cgb("spsw-mode0-cgbBCE.gb");
}

#[test]
fn spsw_stop_prefetch() {
    run_register_cgb("spsw-stop-prefetch-cgbBCE.gb");
}

#[test]
fn spsw_tima() {
    run_register_cgb("spsw-tima-cgbBC.gb");
}

// stat-interrupt/
#[test]
fn stat_int() {
    run_register_shared("stat-int-dmgC-cgbBCE.gb");
}

#[test]
fn stat_int_ncm() {
    run_register_cgb("stat-int-ncmBCE.gb");
}

// stat-mode/
#[test]
fn stat_mode() {
    run_register_shared("stat-mode-dmgC-cgbBC.gb");
}

#[test]
fn stat_mode_ds() {
    run_register_cgb("stat-mode-ds-cgbBCE.gb");
}

#[test]
fn stat_mode_ncm() {
    run_register_cgb("stat-mode-ncmBC.gb");
}

// stat-mode-sprites/
#[test]
fn stat_mode_sprites() {
    run_register_shared("stat-mode-sprites-dmgC-cgbBCE.gb");
}

#[test]
fn stat_mode_sprites_ds() {
    run_register_cgb("stat-mode-sprites-ds-cgbBCE.gb");
}

// stat-mode-window/
#[test]
fn stat_mode_window() {
    run_register_cgb("stat-mode-window-cgbBCE.gb");
}

#[test]
fn stat_mode_window_ds() {
    run_register_cgb("stat-mode-window-ds-cgbBCE.gb");
}

#[test]
fn stat_mode_window_ncm() {
    run_register_cgb("stat-mode-window-ncmBCE.gb");
}

// vram/
#[test]
fn vram_read() {
    run_register_cgb("vram-read-cgbBCE.gb");
}

#[test]
fn vram_read_ncm() {
    run_register_cgb("vram-read-ncmBCE.gb");
}

// m3-bg-bgp/ — screenshot (ROM lives in gb crate, reference here)
#[test]
fn m3_bg_bgp() {
    run_screen_shared("m3-bg-bgp.gb", "m3-bg-bgp-ncmBC.png");
}

// m3-bg-lcdc/ — CGB variants
#[test]
fn m3_bg_lcdc() {
    run_screen_cgb("m3-bg-lcdc.gb", "m3-bg-lcdc-cgbBCE.png");
}

#[test]
fn m3_bg_lcdc_ds() {
    run_screen_cgb("m3-bg-lcdc-ds.gb", "m3-bg-lcdc-ds-cgbBCE.png");
}

// m3-bg-scx/
#[test]
fn m3_bg_scx() {
    run_screen_cgb("m3-bg-scx.gb", "m3-bg-scx-cgbBCE.png");
}

#[test]
fn m3_bg_scx_ds() {
    run_screen_cgb("m3-bg-scx-ds.gb", "m3-bg-scx-ds-cgbBCE.png");
}
