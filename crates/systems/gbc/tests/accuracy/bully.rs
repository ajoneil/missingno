//! BullyGB. Per c-sp howto, the test fails on DMG-C with "Bad Echo RAM
//! Reads" but passes on CGB hardware. Our DMG suite uses the `-dmg.png`
//! reference taken from a passing DMG-style render; we expect the same
//! frame on CGB under our fixed greyscale palette.

use crate::common;
use crate::common::System;

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

    common::assert_screen_matches("Bully", &gbc.screen_greyscale(), "bully/bully-dmg.png");
}
