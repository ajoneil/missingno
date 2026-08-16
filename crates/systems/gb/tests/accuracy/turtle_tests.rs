use crate::common;

fn run_turtle_test(rom_name: &str) {
    let rom_path = format!("turtle-tests/{rom_name}.gb");
    let reference_path = format!("turtle-tests/{rom_name}-dmg.png");

    let mut run = common::load_rom(&rom_path);
    // TurtleTests display results after ~30 frames; don't terminate with a loop
    common::run_frames(&mut run, 30);

    let actual = common::screen_to_greyscale(run.gb.screen());
    let expected = common::load_reference_png(&reference_path);

    common::assert_pixels_match(
        &format!("TurtleTest {rom_name}"),
        &actual,
        &expected,
        160,
        10,
        common::hex_byte,
    );
}

#[test]
fn window_y_trigger() {
    run_turtle_test("window_y_trigger");
}

#[test]
fn window_y_trigger_wx_offscreen() {
    run_turtle_test("window_y_trigger_wx_offscreen");
}
