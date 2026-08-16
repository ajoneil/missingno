use crate::common;

#[test]
fn mbc3_tester() {
    let mut run = common::load_rom("mbc3-tester/mbc3-tester.gb");
    // MBC3 tester loops indefinitely; the bank walk takes ~40 frames to finish.
    common::run_frames(&mut run, 60);

    let actual = common::screen_to_greyscale(run.gb.screen());
    let expected = common::load_reference_png("mbc3-tester/mbc3-tester-dmg.png");

    common::assert_pixels_match("MBC3 tester", &actual, &expected, 160, 10, common::hex_byte);
}
