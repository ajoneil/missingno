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

    common::assert_pixels_match(
        "Strikethrough",
        &actual,
        &expected,
        160,
        10,
        common::hex_byte,
    );
}
