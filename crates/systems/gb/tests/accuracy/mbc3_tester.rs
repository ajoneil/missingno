use crate::common;
use crate::common::System;

#[test]
fn mbc3_tester() {
    let mut run = common::load_rom("mbc3-tester/mbc3-tester.gb");
    // MBC3 tester loops indefinitely; the bank walk takes ~40 frames to finish.
    common::run_frames(&mut run, 60);

    common::assert_screen_matches(
        "MBC3 tester",
        &run.screen_greyscale(),
        "mbc3-tester/mbc3-tester-dmg.png",
    );
}
