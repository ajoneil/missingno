//! cgb-acid-hell by Matt Currie. More demanding CGB PPU edge cases
//! than cgb-acid2.
//!
//! Expected to fail until CGB PPU support lands.

use crate::common;

#[test]
fn cgb_acid_hell() {
    let mut gbc = common::load_cgb_rom("cgb-acid-hell/cgb-acid-hell.gbc");
    let found_breakpoint = common::run_until_breakpoint(&mut gbc, 600);
    assert!(
        found_breakpoint,
        "cgb-acid-hell timed out without reaching LD B,B breakpoint"
    );

    let actual = gbc.screen().to_greyscale_bytes();
    let expected = common::load_cgb_reference_png("cgb-acid-hell/cgb-acid-hell.png");

    common::assert_pixels_match(
        "cgb-acid-hell",
        &actual,
        &expected,
        160,
        10,
        common::hex_byte,
    );
}
