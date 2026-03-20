use crate::common;

#[test]
fn strikethrough() {
    let mut gb = common::load_rom("strikethrough/strikethrough.gb");
    // Strikethrough displays results after ~0.5s; doesn't terminate with a loop
    common::run_frames(&mut gb, 30);

    let actual = common::screen_to_greyscale(gb.screen());
    let expected = common::load_reference_png("strikethrough/strikethrough-dmg.png");

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
        "Strikethrough: {mismatches} pixel mismatches"
    );
}
