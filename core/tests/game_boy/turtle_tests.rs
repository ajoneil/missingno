use crate::common;

fn run_turtle_test(rom_name: &str) {
    let rom_path = format!("turtle-tests/{rom_name}.gb");
    let reference_path = format!("turtle-tests/{rom_name}-dmg.png");

    let mut gb = common::load_rom(&rom_path);
    // TurtleTests display results after ~30 frames; don't terminate with a loop
    common::run_frames(&mut gb, 30);

    let actual = common::screen_to_greyscale(gb.screen());
    let expected = common::load_reference_png(&reference_path);

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
        "TurtleTest {rom_name}: {mismatches} pixel mismatches"
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
