use crate::common;

#[test]
fn firstwhite() {
    let mut run = common::load_rom("little-things-gb/firstwhite.gb");
    // Result is visible nearly immediately; doesn't terminate with a loop
    common::run_frames(&mut run, 30);

    let expected = common::load_reference_png("little-things-gb/firstwhite-dmg.png");

    // The ROM cycles LCDC.7 once per frame and relies on the first frame
    // after each LCD-on being uncommitted to the LCD. Check 10 consecutive
    // frames so a single text-leaking frame fails the test even when
    // other frames are white.
    for frame in 0..10 {
        while !run.step().new_screen {}
        let actual = common::screen_to_greyscale(run.gb.screen());

        common::assert_pixels_match(
            &format!("firstwhite frame {frame}"),
            &actual,
            &expected,
            160,
            10,
            common::hex_byte,
        );
    }
}
