//! BullyGB. Per c-sp howto, the test fails on DMG-C with "Bad Echo RAM
//! Reads" but passes on CGB hardware. Our DMG suite uses the `-dmg.png`
//! reference taken from a passing DMG-style render; we expect the same
//! frame on CGB under our fixed greyscale palette.

use crate::common;

#[test]
fn bully() {
    let mut gbc = common::load_rom("bully/bully.gb");
    let found_loop = common::run_until_infinite_loop(&mut gbc, 60);
    assert!(found_loop, "Bully timed out without reaching infinite loop");

    // Bully enables the LCD one instruction before its lock-up `JR @`,
    // so the JR-2 predicate fires on a mid-scanout frame. Advance a
    // couple more frames so the captured screen is the stable rendering.
    for _ in 0..2 {
        while !gbc.step().new_screen {}
    }

    let actual = gbc.screen().to_greyscale_bytes();
    let expected = common::load_reference_png("bully/bully-dmg.png");

    common::assert_pixels_match("Bully", &actual, &expected, 160, 10, common::hex_byte);
}
