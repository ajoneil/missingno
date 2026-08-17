//! little-things-gb (firstwhite). The howto says firstwhite works on all
//! Game Boys except SGB. We skip Telling LYs since it requires input
//! emulation.

use crate::common;
use crate::common::System;

#[test]
fn firstwhite() {
    let mut gbc = common::load_rom("little-things-gb/firstwhite.gb");
    common::run_frames(&mut gbc, 30);

    let reference = "little-things-gb/firstwhite-dmg.png";

    // The ROM cycles LCDC.7 once per frame and relies on the first frame
    // after each LCD-on being uncommitted to the LCD. Check 10 consecutive
    // frames so a single text-leaking frame fails the test.
    for frame in 0..10 {
        while !gbc.step().new_screen {}
        common::assert_screen_matches(
            &format!("firstwhite frame {frame}"),
            &gbc.screen_greyscale(),
            reference,
        );
    }
}
