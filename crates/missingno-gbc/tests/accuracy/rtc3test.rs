//! MBC3 real-time clock tests by ax6.
//!
//! Per the c-sp howto, three sub-tests are selected at startup via
//! button presses, then run for a fixed emulated duration:
//!
//! | sub-test           | button sequence | duration (seconds) |
//! |--------------------|-----------------|--------------------|
//! | basic tests        | A               | 13                 |
//! | range tests        | down, A         | 8                  |
//! | sub-second writes  | down, down, A   | 26                 |
//!
//! Compared against the CGB reference PNG.

use crate::common;
use missingno_gb::joypad::{Button, DirectionalPad};
use missingno_gbc::GameBoyColor;

fn run_frames(gbc: &mut GameBoyColor, frames: u32) {
    for _ in 0..frames {
        while !gbc.step().new_screen {}
    }
}

fn press_button(gbc: &mut GameBoyColor, button: Button) {
    gbc.press_button(button);
    run_frames(gbc, 5);
    gbc.release_button(button);
    run_frames(gbc, 5);
}

fn check_screen(gbc: &GameBoyColor, reference: &str) {
    let actual = gbc.screen().to_greyscale_bytes();
    let expected = common::load_cgb_reference_png(reference);
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
    assert_eq!(mismatches, 0, "rtc3test: {mismatches} pixel mismatches vs {reference}");
}

#[test]
fn basic_tests() {
    let mut gbc = common::load_cgb_rom("rtc3test/rtc3test.gb");
    // Let the menu render first.
    run_frames(&mut gbc, 30);
    press_button(&mut gbc, Button::A);
    // 13 emulated seconds = ~780 frames.
    run_frames(&mut gbc, 780);
    check_screen(&gbc, "rtc3test/rtc3test-basic-tests-cgb.png");
}

#[test]
fn range_tests() {
    let mut gbc = common::load_cgb_rom("rtc3test/rtc3test.gb");
    run_frames(&mut gbc, 30);
    press_button(&mut gbc, Button::DirectionalPad(DirectionalPad::Down));
    press_button(&mut gbc, Button::A);
    // 8 emulated seconds = ~480 frames.
    run_frames(&mut gbc, 480);
    check_screen(&gbc, "rtc3test/rtc3test-range-tests-cgb.png");
}

#[test]
fn sub_second_writes() {
    let mut gbc = common::load_cgb_rom("rtc3test/rtc3test.gb");
    run_frames(&mut gbc, 30);
    press_button(&mut gbc, Button::DirectionalPad(DirectionalPad::Down));
    press_button(&mut gbc, Button::DirectionalPad(DirectionalPad::Down));
    press_button(&mut gbc, Button::A);
    // 26 emulated seconds = ~1560 frames.
    run_frames(&mut gbc, 1560);
    check_screen(&gbc, "rtc3test/rtc3test-sub-second-writes-cgb.png");
}
