//! dmg-acid2 run on the CGB core. In DMG-compatibility mode the boot
//! palette colourises the image, so it's compared in full RGB against
//! the CGB reference (`dmg-acid2-cgb.png`).

use crate::common;

#[test]
fn dmg_acid2() {
    let mut gbc = common::load_rom("dmg-acid2/dmg-acid2.gb");
    for _ in 0..5 {
        while !gbc.step().new_screen {}
    }

    let actual = gbc.screen().to_rgb_bytes();
    let expected = common::load_reference_png_rgb("dmg-acid2/dmg-acid2-cgb.png");

    let mut mismatches = 0;
    for (i, (a, e)) in actual.chunks(3).zip(expected.chunks(3)).enumerate() {
        if a != e {
            let (x, y) = (i % 160, i / 160);
            if mismatches < 10 {
                eprintln!(
                    "Pixel mismatch at ({x}, {y}): got #{:02X}{:02X}{:02X}, expected #{:02X}{:02X}{:02X}",
                    a[0], a[1], a[2], e[0], e[1], e[2]
                );
            }
            mismatches += 1;
        }
    }

    assert_eq!(mismatches, 0, "dmg-acid2: {mismatches} pixel mismatches");
}
