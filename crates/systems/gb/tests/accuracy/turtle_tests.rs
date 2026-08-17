use crate::common;

fn run_turtle_test(rom_name: &str) {
    let mut run = common::load_rom(&format!("turtle-tests/{rom_name}.gb"));
    common::assert_turtle_test(&mut run, rom_name);
}

#[test]
fn window_y_trigger() {
    run_turtle_test("window_y_trigger");
}

#[test]
fn window_y_trigger_wx_offscreen() {
    run_turtle_test("window_y_trigger_wx_offscreen");
}
