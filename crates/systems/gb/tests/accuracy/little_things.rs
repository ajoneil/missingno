use crate::common;

#[test]
fn firstwhite() {
    let mut run = common::load_rom("little-things-gb/firstwhite.gb");
    // Result is visible nearly immediately; doesn't terminate with a loop
    common::run_frames(&mut run, 30);

    let expected = common::load_reference_png("little-things-gb/firstwhite-dmg.png");

    // The ROM cycles LCDC.7 once per frame and relies on the first frame
    // after each LCD-on being uncommitted to the LCD. Check 10 consecutive
    // frames so a single text-leaking frame fails the test even when
    // other frames are white.
    for frame in 0..10 {
        while !run.step().new_screen {}
        let actual = common::screen_to_greyscale(run.gb.screen());

        let mut mismatches = 0;
        for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
            if a != e {
                if mismatches < 10 {
                    let (x, y) = (i % 160, i / 160);
                    eprintln!(
                        "frame {frame}: pixel mismatch at ({x}, {y}): got 0x{a:02X}, expected 0x{e:02X}"
                    );
                }
                mismatches += 1;
            }
        }
        assert_eq!(
            mismatches, 0,
            "firstwhite frame {frame}: {mismatches} pixel mismatches"
        );
    }
}
