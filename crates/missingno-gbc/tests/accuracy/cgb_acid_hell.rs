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

    let mut mismatches = 0;
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        if a != e {
            if mismatches < 10 {
                let (x, y) = (i % 160, i / 160);
                eprintln!("Pixel mismatch at ({x}, {y}): got 0x{a:02X}, expected 0x{e:02X}");
            }
            mismatches += 1;
        }
    }

    assert_eq!(
        mismatches, 0,
        "cgb-acid-hell: {mismatches} pixel mismatches"
    );
}
