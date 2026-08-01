use crate::common;

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

    let actual = common::screen_to_greyscale(run.gb.screen());
    let expected = common::load_reference_png("bully/bully-dmg.png");

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

    assert_eq!(mismatches, 0, "Bully: {mismatches} pixel mismatches");
}
