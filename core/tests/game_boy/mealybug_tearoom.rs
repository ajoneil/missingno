use crate::common;

fn run_mealybug_test(rom_name: &str) {
    let rom_path = format!("mealybug-tearoom/{rom_name}.gb");
    let reference_path = format!("mealybug-tearoom/{rom_name}-expected.png");

    let mut gb = common::load_rom(&rom_path);
    let found_breakpoint = common::run_until_breakpoint(&mut gb, 1200);
    assert!(
        found_breakpoint,
        "Mealybug test {rom_name} timed out without reaching LD B,B breakpoint"
    );

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
        "Mealybug test {rom_name}: {mismatches} pixel mismatches"
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
