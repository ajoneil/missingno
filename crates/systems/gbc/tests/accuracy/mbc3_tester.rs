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

    common::assert_pixels_match("MBC3 tester", &actual, &expected, 160, 10, common::hex_byte);
}
