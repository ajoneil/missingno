use crate::common;

#[test]
fn dmg_acid2() {
    let mut run = common::load_rom("dmg-acid2/dmg-acid2.gb");
    for _ in 0..5 {
        while !run.step().new_screen {}
    }

    let actual = common::screen_to_greyscale(run.gb.screen());
    let expected = common::load_reference_png("dmg-acid2/dmg-acid2-dmg.png");

    common::assert_pixels_match(
        "dmg-acid2",
        &actual,
        &expected,
        160,
        usize::MAX,
        common::hex_byte,
    );
}
