//! little-things-gb (firstwhite). The howto says firstwhite works on all
//! Game Boys except SGB. We skip Telling LYs since it requires input
//! emulation.

use crate::common;

#[test]
fn firstwhite() {
    let mut gbc = common::load_rom("little-things-gb/firstwhite.gb");
    common::run_frames(&mut gbc, 30);

    let expected = common::load_reference_png("little-things-gb/firstwhite-dmg.png");

    // The ROM cycles LCDC.7 once per frame and relies on the first frame
    // after each LCD-on being uncommitted to the LCD. Check 10 consecutive
    // frames so a single text-leaking frame fails the test.
    for frame in 0..10 {
        while !gbc.step().new_screen {}
        let actual = gbc.screen().to_greyscale_bytes();

        common::assert_pixels_match(
            &format!("firstwhite frame {frame}"),
            &actual,
            &expected,
            160,
            10,
            common::hex_byte,
        );
    }
}
