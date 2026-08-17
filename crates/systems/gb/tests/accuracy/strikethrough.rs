use crate::common;
use crate::common::System;

#[test]
fn strikethrough() {
    let mut run = common::load_rom("strikethrough/strikethrough.gb");
    // Strikethrough displays results after ~0.5s; doesn't terminate with a loop
    common::run_frames(&mut run, 30);

    common::assert_screen_matches(
        "Strikethrough",
        &run.screen_greyscale(),
        "strikethrough/strikethrough-dmg.png",
    );
}
