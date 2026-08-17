//! cgb-acid-hell by Matt Currie. More demanding CGB PPU edge cases
//! than cgb-acid2.
//!
//! Expected to fail until CGB PPU support lands.

use crate::common;
use crate::common::System;

#[test]
fn cgb_acid_hell() {
    let mut gbc = common::load_cgb_rom("cgb-acid-hell/cgb-acid-hell.gbc");
    let found_breakpoint = common::run_until_breakpoint(&mut gbc, 600);
    assert!(
        found_breakpoint,
        "cgb-acid-hell timed out without reaching LD B,B breakpoint"
    );

    common::assert_cgb_screen_matches(
        "cgb-acid-hell",
        &gbc.screen_greyscale(),
        "cgb-acid-hell/cgb-acid-hell.png",
    );
}
