use crate::common;
use crate::common::System;

#[test]
fn dmg_acid2() {
    let mut run = common::load_rom("dmg-acid2/dmg-acid2.gb");
    for _ in 0..5 {
        while !run.step().new_screen {}
    }

    common::assert_screen_matches(
        "dmg-acid2",
        &run.screen_greyscale(),
        "dmg-acid2/dmg-acid2-dmg.png",
    );
}
