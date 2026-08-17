use crate::common;

fn run_scribbltest(rom_name: &str, timeout_frames: u32) {
    let mut run = common::load_rom(&format!("scribbltests/{rom_name}.gb"));
    common::assert_scribbltest(&mut run, rom_name, timeout_frames);
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
