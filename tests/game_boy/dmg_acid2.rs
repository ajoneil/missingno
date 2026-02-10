use crate::common;

#[test]
fn dmg_acid2() {
    let mut gb = common::load_rom("dmg-acid2/dmg-acid2.gb");
    let found_loop = common::run_until_infinite_loop(&mut gb, 600);
    assert!(
        found_loop,
        "dmg-acid2 timed out without reaching infinite loop"
    );

    let actual = common::screen_to_greyscale(gb.screen());
    let expected = common::load_reference_png("dmg-acid2/dmg-acid2-dmg.png");

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

    assert_eq!(mismatches, 0, "dmg-acid2: {mismatches} pixel mismatches");
}
