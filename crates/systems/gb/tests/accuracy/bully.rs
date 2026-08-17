use crate::common;
use crate::common::System;

#[test]
fn bully() {
    let mut run = common::load_rom("bully/bully.gb");
    // Bully needs ~0.5s emulated time (~30 frames)
    let found_loop = common::run_until_infinite_loop(&mut run, 60);
    assert!(found_loop, "Bully timed out without reaching infinite loop");

    // Bully enables the LCD one instruction before its lock-up `JR @`
    // (BullyGB src/main.asm:151-154), so the JR-2 predicate fires on a
    // mid-scanout frame. Advance a couple more frames so the captured
    // screen is the stable rendering the reference PNG was taken from.
    for _ in 0..2 {
        while !run.step().new_screen {}
    }

    common::assert_screen_matches("Bully", &run.screen_greyscale(), "bully/bully-dmg.png");
}
