//! MBC3 bank tester. The c-sp howto notes CGB runs in compatibility mode;
//! the screen output matches the DMG reference under our fixed greyscale
//! palette, so we reuse the `-dmg.png` reference.

use crate::common;
use crate::common::System;

#[test]
fn mbc3_tester() {
    let mut gbc = common::load_rom("mbc3-tester/mbc3-tester.gb");
    common::run_frames(&mut gbc, 60);

    common::assert_screen_matches(
        "MBC3 tester",
        &gbc.screen_greyscale(),
        "mbc3-tester/mbc3-tester-dmg.png",
    );
}
