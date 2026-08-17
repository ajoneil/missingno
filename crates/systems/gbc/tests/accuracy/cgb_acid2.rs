//! cgb-acid2 by Matt Currie. Exercises CGB-specific PPU features:
//! CGB BG/OBJ palettes ($FF68-$FF6B), tile attributes (bank 1 of
//! VRAM tilemap), BG/OBJ priority, etc.
//!
//! Expected to fail until CGB PPU support lands. The test runs to
//! completion via `LD B,B` then we compare against the reference PNG
//! shipped with the ROM.

use crate::common;
use crate::common::System;

#[test]
fn cgb_acid2() {
    let mut gbc = common::load_cgb_rom("cgb-acid2/cgb-acid2.gbc");
    let found_breakpoint = common::run_until_breakpoint(&mut gbc, 600);
    assert!(
        found_breakpoint,
        "cgb-acid2 timed out without reaching LD B,B breakpoint"
    );

    common::assert_cgb_screen_matches(
        "cgb-acid2",
        &gbc.screen_greyscale(),
        "cgb-acid2/cgb-acid2.png",
    );
}
