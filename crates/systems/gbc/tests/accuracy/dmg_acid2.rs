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

    let actual = common::rgb_pixels(&gbc.screen().to_rgb_bytes());
    let expected = common::load_reference_png_rgb("dmg-acid2/dmg-acid2-cgb.png");

    common::assert_pixels_match(
        "dmg-acid2",
        &actual,
        &expected,
        160,
        10,
        common::debug_value,
    );
}
