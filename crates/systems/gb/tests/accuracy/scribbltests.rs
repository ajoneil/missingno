use crate::common;

fn run_scribbltest(rom_name: &str, timeout_frames: u32) {
    let rom_path = format!("scribbltests/{rom_name}.gb");
    let reference_path = format!("scribbltests/{rom_name}-dmg.png");

    let mut run = common::load_rom(&rom_path);
    let found_breakpoint = common::run_until_breakpoint(&mut run, timeout_frames);
    assert!(
        found_breakpoint,
        "Scribbltest {rom_name} timed out without reaching LD B,B breakpoint"
    );

    let actual = common::screen_to_greyscale(run.gb.screen());
    let expected = common::load_reference_png(&reference_path);

    common::assert_pixels_match(
        &format!("Scribbltest {rom_name}"),
        &actual,
        &expected,
        160,
        10,
        common::hex_byte,
    );
}

#[test]
fn lycscx() {
    run_scribbltest("lycscx", 30);
}

#[test]
fn lycscy() {
    run_scribbltest("lycscy", 30);
}

#[test]
fn palettely() {
    run_scribbltest("palettely", 30);
}

#[test]
fn scxly() {
    run_scribbltest("scxly", 30);
}

#[test]
fn statcount_auto() {
    // statcount_auto needs ~270 frames (~4.5 seconds emulated)
    run_scribbltest("statcount-auto", 300);
}
