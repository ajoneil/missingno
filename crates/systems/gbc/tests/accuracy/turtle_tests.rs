//! Turtle Tests — c-sp howto says probably runs on any DMG and CGB.

use crate::common;

fn run_turtle_test(rom_name: &str) {
    let mut gbc = common::load_rom(&format!("turtle-tests/{rom_name}.gb"));
    common::assert_turtle_test(&mut gbc, rom_name);
}

#[test]
fn window_y_trigger() {
    run_turtle_test("window_y_trigger");
}

#[test]
fn window_y_trigger_wx_offscreen() {
    run_turtle_test("window_y_trigger_wx_offscreen");
}
