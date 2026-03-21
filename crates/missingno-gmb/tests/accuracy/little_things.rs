use crate::common;

#[test]
fn firstwhite() {
    let mut run = common::load_rom("little-things-gb/firstwhite.gb");
    // Result is visible nearly immediately; doesn't terminate with a loop
    common::run_frames(&mut run, 30);

    let actual = common::screen_to_greyscale(run.gb.screen());
    let expected = common::load_reference_png("little-things-gb/firstwhite-dmg.png");

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

    assert_eq!(mismatches, 0, "firstwhite: {mismatches} pixel mismatches");
}
