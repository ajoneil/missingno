//! Mealybug Tearoom Tests — PPU mid-scanline change tests.
//!
//! Compared against the CGB-C reference screenshots (`_cgb_c.png`)
//! shipped in c-sp v7. ROMs come from two places:
//!
//! - Shared ROMs (same as DMG suite): live in `missingno-gb`'s roms
//!   tree, loaded via [`common::load_rom`]. Compared against the
//!   CGB-C reference in this crate's roms dir.
//! - `_change2` variants: newer CGB-C-only test variants, live in
//!   this crate's roms dir, loaded via [`common::load_cgb_rom`].
//!
//! All expected to fail until the CGB PPU lands.

use crate::common;
use missingno_gbc::GameBoyColor;

fn run_test(rom: GameBoyColor, rom_name: &str) {
    let mut gbc = rom;
    let found_breakpoint = common::run_until_breakpoint(&mut gbc, 1200);
    assert!(
        found_breakpoint,
        "Mealybug test {rom_name} timed out without reaching LD B,B breakpoint"
    );

    let actual = gbc.screen().to_rgb_bytes();
    let reference = format!("mealybug-tearoom/{rom_name}_cgb_c.png");
    let expected = common::load_cgb_reference_png_rgb(&reference);

    let mut mismatches = 0;
    for (i, (a, e)) in actual.chunks_exact(3).zip(expected.chunks_exact(3)).enumerate() {
        if a != e {
            if mismatches < 10 {
                let (x, y) = (i % 160, i / 160);
                eprintln!("Pixel mismatch at ({x}, {y}): got {a:?}, expected {e:?}");
            }
            mismatches += 1;
        }
    }

    assert_eq!(
        mismatches, 0,
        "Mealybug test {rom_name}: {mismatches} pixel mismatches"
    );
}

fn run_shared(rom_name: &str) {
    let rom = common::load_rom(&format!("mealybug-tearoom/{rom_name}.gb"));
    run_test(rom, rom_name);
}

fn run_change2(rom_name: &str) {
    let rom = common::load_cgb_rom(&format!("mealybug-tearoom/{rom_name}.gb"));
    run_test(rom, rom_name);
}

macro_rules! shared {
    ($name:ident) => {
        #[test]
        fn $name() {
            run_shared(stringify!($name));
        }
    };
}

macro_rules! change2 {
    ($name:ident) => {
        #[test]
        fn $name() {
            run_change2(stringify!($name));
        }
    };
}

// Existing mealybug ROMs with CGB-C references shipped in c-sp v7.
shared!(m2_win_en_toggle);
shared!(m3_bgp_change);
shared!(m3_bgp_change_sprites);
shared!(m3_lcdc_bg_en_change);
shared!(m3_lcdc_bg_map_change);
shared!(m3_lcdc_obj_en_change);
shared!(m3_lcdc_obj_en_change_variant);
shared!(m3_lcdc_obj_size_change);
shared!(m3_lcdc_obj_size_change_scx);
shared!(m3_lcdc_tile_sel_change);
shared!(m3_lcdc_tile_sel_win_change);
shared!(m3_lcdc_win_en_change_multiple);
shared!(m3_lcdc_win_map_change);
shared!(m3_obp0_change);
shared!(m3_scx_high_5_bits);
shared!(m3_scx_low_3_bits);
shared!(m3_scy_change);
shared!(m3_window_timing);
shared!(m3_window_timing_wx_0);
shared!(m3_wx_4_change_sprites);
shared!(m3_lcdc_win_en_change_multiple_wx);
shared!(m3_wx_4_change);
shared!(m3_wx_5_change);
shared!(m3_wx_6_change);

// CGB-C-only `_change2` variants.
change2!(m3_lcdc_bg_en_change2);
change2!(m3_lcdc_bg_map_change2);
change2!(m3_lcdc_tile_sel_change2);
change2!(m3_lcdc_tile_sel_win_change2);
change2!(m3_lcdc_win_map_change2);
change2!(m3_scx_high_5_bits_change2);
change2!(m3_scy_change2);
