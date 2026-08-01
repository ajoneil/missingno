//! Strikethrough — confirmed by c-sp howto to work on both DMG and CGB. The
//! ROM detects the CGB (A=$11) and inverts the display, so the CGB run is
//! compared against the CGB reference, not the DMG one.

use crate::common;

#[test]
fn strikethrough() {
    let mut gbc = common::load_rom("strikethrough/strikethrough.gb");
    common::run_frames(&mut gbc, 30);

    let actual = gbc.screen().to_greyscale_bytes();
    let expected = common::load_cgb_reference_png("strikethrough/strikethrough-cgb.png");

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
