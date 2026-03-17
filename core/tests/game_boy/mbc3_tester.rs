use crate::common;

#[test]
fn mbc3_tester() {
    let mut gb = common::load_rom("mbc3-tester/mbc3-tester.gb");
    // MBC3 tester loops indefinitely; result visible after 40 frames
    common::run_frames(&mut gb, 40);

    let actual = common::screen_to_greyscale(gb.screen());
    let expected = common::load_reference_png("mbc3-tester/mbc3-tester-dmg.png");

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
        "MBC3 tester: {mismatches} pixel mismatches"
    );
}
