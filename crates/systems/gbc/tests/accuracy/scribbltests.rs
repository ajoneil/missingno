//! Scribbltests — c-sp howto says verified on MGB and CPU CGB D.

use crate::common;

fn run_scribbltest(rom_name: &str, timeout_frames: u32) {
    let rom_path = format!("scribbltests/{rom_name}.gb");
    let reference_path = format!("scribbltests/{rom_name}-dmg.png");

    let mut gbc = common::load_rom(&rom_path);
    let found_breakpoint = common::run_until_breakpoint(&mut gbc, timeout_frames);
    assert!(
        found_breakpoint,
        "Scribbltest {rom_name} timed out without reaching LD B,B breakpoint"
    );

    let actual = gbc.screen().to_greyscale_bytes();
    let expected = common::load_reference_png(&reference_path);

    common::assert_pixels_match(
        &format!("Scribbltest {rom_name}"),
        &actual,
        &expected,
        160,
        10,
        common::hex_byte,
    );
}

#[test]
fn lycscx() {
    run_scribbltest("lycscx", 30);
}

#[test]
fn lycscy() {
    run_scribbltest("lycscy", 30);
}

#[test]
fn palettely() {
    run_scribbltest("palettely", 30);
}

#[test]
fn scxly() {
    run_scribbltest("scxly", 30);
}

#[test]
fn statcount_auto() {
    run_scribbltest("statcount-auto", 300);
}
