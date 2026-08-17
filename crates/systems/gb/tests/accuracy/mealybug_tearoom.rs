use crate::common;
use crate::common::System;

fn run_mealybug_test(rom_name: &str) {
    let rom_path = format!("mealybug-tearoom/{rom_name}.gb");
    let reference_path = format!("mealybug-tearoom/{rom_name}-expected.png");

    let mut run = common::load_rom(&rom_path);
    let found_breakpoint = common::run_until_breakpoint(&mut run, 1200);
    assert!(
        found_breakpoint,
        "Mealybug test {rom_name} timed out without reaching LD B,B breakpoint"
    );

    common::assert_screen_matches(
        &format!("Mealybug test {rom_name}"),
        &run.screen_greyscale(),
        &reference_path,
    );
}

macro_rules! mealybug_test {
    ($name:ident) => {
        #[test]
        fn $name() {
            run_mealybug_test(stringify!($name));
        }
    };
}

// Mode 2 — OAM scan register changes
mealybug_test!(m2_win_en_toggle);

// Mode 3 — LCDC register changes mid-scanline
mealybug_test!(m3_lcdc_bg_en_change);
mealybug_test!(m3_lcdc_bg_map_change);
mealybug_test!(m3_lcdc_obj_en_change);
mealybug_test!(m3_lcdc_obj_en_change_variant);
mealybug_test!(m3_lcdc_obj_size_change);
mealybug_test!(m3_lcdc_obj_size_change_scx);
mealybug_test!(m3_lcdc_tile_sel_change);
mealybug_test!(m3_lcdc_tile_sel_win_change);
mealybug_test!(m3_lcdc_win_en_change_multiple);
mealybug_test!(m3_lcdc_win_en_change_multiple_wx);
mealybug_test!(m3_lcdc_win_map_change);

// Mode 3 — palette changes mid-scanline
mealybug_test!(m3_bgp_change);
mealybug_test!(m3_bgp_change_sprites);
mealybug_test!(m3_obp0_change);

// Mode 3 — scroll register changes mid-scanline
mealybug_test!(m3_scx_high_5_bits);
mealybug_test!(m3_scx_low_3_bits);
mealybug_test!(m3_scy_change);

// Mode 3 — window timing
mealybug_test!(m3_window_timing);
mealybug_test!(m3_window_timing_wx_0);
mealybug_test!(m3_wx_4_change);
mealybug_test!(m3_wx_4_change_sprites);
mealybug_test!(m3_wx_5_change);
mealybug_test!(m3_wx_6_change);
