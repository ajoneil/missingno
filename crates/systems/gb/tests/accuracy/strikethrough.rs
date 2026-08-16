use crate::common;

#[test]
fn strikethrough() {
    let mut run = common::load_rom("strikethrough/strikethrough.gb");
    // Strikethrough displays results after ~0.5s; doesn't terminate with a loop
    common::run_frames(&mut run, 30);

    let actual = common::screen_to_greyscale(run.gb.screen());
    let expected = common::load_reference_png("strikethrough/strikethrough-dmg.png");

    common::assert_pixels_match(
        "Strikethrough",
        &actual,
        &expected,
        160,
        10,
        common::hex_byte,
    );
}
