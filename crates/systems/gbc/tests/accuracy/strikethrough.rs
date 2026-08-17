//! Strikethrough — confirmed by c-sp howto to work on both DMG and CGB. The
//! ROM detects the CGB (A=$11) and inverts the display, so the CGB run is
//! compared against the CGB reference, not the DMG one.

use crate::common;
use crate::common::System;

#[test]
fn strikethrough() {
    let mut gbc = common::load_rom("strikethrough/strikethrough.gb");
    common::run_frames(&mut gbc, 30);

    common::assert_cgb_screen_matches(
        "Strikethrough",
        &gbc.screen_greyscale(),
        "strikethrough/strikethrough-cgb.png",
    );
}
