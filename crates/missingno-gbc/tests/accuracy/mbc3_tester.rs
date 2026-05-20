//! MBC3 bank tester. The c-sp howto notes CGB runs in compatibility mode;
//! the screen output matches the DMG reference under our fixed greyscale
//! palette, so we reuse the `-dmg.png` reference.

use crate::common;

#[test]
fn mbc3_tester() {
    let mut gbc = common::load_rom("mbc3-tester/mbc3-tester.gb");
    common::run_frames(&mut gbc, 60);

    let actual = gbc.screen().to_greyscale_bytes();
    let expected = common::load_reference_png("mbc3-tester/mbc3-tester-dmg.png");

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

    assert_eq!(mismatches, 0, "MBC3 tester: {mismatches} pixel mismatches");
}
