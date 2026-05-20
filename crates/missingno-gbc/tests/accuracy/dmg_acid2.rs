//! dmg-acid2. Compared against the CGB-compatibility-mode reference
//! (`dmg-acid2-cgb.png`) — running this DMG ROM on CGB produces a
//! different image because the CGB boot ROM injects a compat-mode
//! palette and CGB hardware doesn't have the DMG OBJ priority bug
//! that this test exercises. Expected to fail until those CGB
//! features land.

use crate::common;

#[test]
fn dmg_acid2() {
    let mut gbc = common::load_rom("dmg-acid2/dmg-acid2.gb");
    for _ in 0..5 {
        while !gbc.step().new_screen {}
    }

    let actual = gbc.screen().to_greyscale_bytes();
    let expected = common::load_reference_png("dmg-acid2/dmg-acid2-cgb.png");

    let mut mismatches = 0;
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        if a != e {
            let (x, y) = (i % 160, i / 160);
            if mismatches < 10 {
                eprintln!("Pixel mismatch at ({x}, {y}): got 0x{a:02X}, expected 0x{e:02X}");
            }
            mismatches += 1;
        }
    }

    assert_eq!(mismatches, 0, "dmg-acid2: {mismatches} pixel mismatches");
}
