use crate::common;

#[test]
fn bully() {
    let mut gb = common::load_rom("bully/bully.gb");
    // Bully needs ~0.5s emulated time (~30 frames)
    let found_loop = common::run_until_infinite_loop(&mut gb, 60);
    assert!(found_loop, "Bully timed out without reaching infinite loop");

    let actual = common::screen_to_greyscale(gb.screen());
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
