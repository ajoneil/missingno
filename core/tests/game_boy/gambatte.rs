use crate::common;

const FRAMES: u32 = 15;

// Gambatte hex digit tile patterns (8x8 pixels each).
// 0 = foreground (0x00 greyscale), 1 = background (0xFF greyscale).
// Derived from gambatte-core testrunner.cpp tileFromChar().
#[rustfmt::skip]
const HEX_TILES: [[u8; 64]; 16] = [
    // 0
    [1,1,1,1,1,1,1,1,
     1,1,0,0,0,0,1,1,
     1,0,1,1,1,1,0,1,
     1,0,1,1,1,1,0,1,
     1,0,1,1,1,1,0,1,
     1,0,1,1,1,1,0,1,
     1,1,0,0,0,0,1,1,
     1,1,1,1,1,1,1,1],
    // 1
    [1,1,1,1,1,1,1,1,
     1,1,1,1,0,1,1,1,
     1,1,1,1,0,1,1,1,
     1,1,1,1,0,1,1,1,
     1,1,1,1,0,1,1,1,
     1,1,1,1,0,1,1,1,
     1,1,1,1,0,1,1,1,
     1,1,1,1,1,1,1,1],
    // 2
    [1,1,1,1,1,1,1,1,
     1,1,0,0,0,0,1,1,
     1,1,1,1,1,1,0,1,
     1,1,0,0,0,0,1,1,
     1,0,1,1,1,1,1,1,
     1,0,1,1,1,1,1,1,
     1,1,0,0,0,0,1,1,
     1,1,1,1,1,1,1,1],
    // 3
    [1,1,1,1,1,1,1,1,
     1,1,0,0,0,0,1,1,
     1,1,1,1,1,1,0,1,
     1,1,0,0,0,0,0,1,
     1,1,1,1,1,1,0,1,
     1,1,1,1,1,1,0,1,
     1,1,0,0,0,0,1,1,
     1,1,1,1,1,1,1,1],
    // 4
    [1,1,1,1,1,1,1,1,
     1,0,1,1,1,1,0,1,
     1,0,1,1,1,1,0,1,
     1,1,0,0,0,0,0,1,
     1,1,1,1,1,1,0,1,
     1,1,1,1,1,1,0,1,
     1,1,1,1,1,1,0,1,
     1,1,1,1,1,1,1,1],
    // 5
    [1,1,1,1,1,1,1,1,
     1,1,0,0,0,0,1,1,
     1,0,1,1,1,1,1,1,
     1,1,0,0,0,0,1,1,
     1,1,1,1,1,1,0,1,
     1,1,1,1,1,1,0,1,
     1,1,0,0,0,0,1,1,
     1,1,1,1,1,1,1,1],
    // 6
    [1,1,1,1,1,1,1,1,
     1,1,0,0,0,0,1,1,
     1,0,1,1,1,1,1,1,
     1,1,0,0,0,0,1,1,
     1,0,1,1,1,1,0,1,
     1,0,1,1,1,1,0,1,
     1,1,0,0,0,0,1,1,
     1,1,1,1,1,1,1,1],
    // 7
    [1,1,1,1,1,1,1,1,
     1,1,0,0,0,0,0,1,
     1,1,1,1,1,0,1,1,
     1,1,1,1,0,1,1,1,
     1,1,1,0,1,1,1,1,
     1,1,1,0,1,1,1,1,
     1,1,1,0,1,1,1,1,
     1,1,1,1,1,1,1,1],
    // 8
    [1,1,1,1,1,1,1,1,
     1,1,0,0,0,0,1,1,
     1,0,1,1,1,1,0,1,
     1,1,0,0,0,0,1,1,
     1,0,1,1,1,1,0,1,
     1,0,1,1,1,1,0,1,
     1,1,0,0,0,0,1,1,
     1,1,1,1,1,1,1,1],
    // 9
    [1,1,1,1,1,1,1,1,
     1,1,0,0,0,0,1,1,
     1,0,1,1,1,1,0,1,
     1,1,0,0,0,0,0,1,
     1,1,1,1,1,1,0,1,
     1,1,1,1,1,1,0,1,
     1,1,0,0,0,0,1,1,
     1,1,1,1,1,1,1,1],
    // A
    [1,1,1,1,1,1,1,1,
     1,1,1,1,0,1,1,1,
     1,1,1,0,1,0,1,1,
     1,1,0,1,1,1,0,1,
     1,1,0,0,0,0,0,1,
     1,0,1,1,1,1,1,0,
     1,0,1,1,1,1,1,0,
     1,1,1,1,1,1,1,1],
    // B
    [1,1,1,1,1,1,1,1,
     1,0,0,0,0,0,0,1,
     1,0,1,1,1,1,1,0,
     1,0,0,0,0,0,0,1,
     1,0,1,1,1,1,1,0,
     1,0,1,1,1,1,1,0,
     1,0,0,0,0,0,0,1,
     1,1,1,1,1,1,1,1],
    // C
    [1,1,1,1,1,1,1,1,
     1,1,0,0,0,0,0,1,
     1,0,1,1,1,1,1,1,
     1,0,1,1,1,1,1,1,
     1,0,1,1,1,1,1,1,
     1,0,1,1,1,1,1,1,
     1,1,0,0,0,0,0,1,
     1,1,1,1,1,1,1,1],
    // D
    [1,1,1,1,1,1,1,1,
     1,0,0,0,0,0,1,1,
     1,0,1,1,1,1,0,1,
     1,0,1,1,1,1,0,1,
     1,0,1,1,1,1,0,1,
     1,0,1,1,1,1,0,1,
     1,0,0,0,0,0,1,1,
     1,1,1,1,1,1,1,1],
    // E
    [1,1,1,1,1,1,1,1,
     1,0,0,0,0,0,0,1,
     1,0,1,1,1,1,1,1,
     1,0,0,0,0,0,0,1,
     1,0,1,1,1,1,1,1,
     1,0,1,1,1,1,1,1,
     1,0,0,0,0,0,0,1,
     1,1,1,1,1,1,1,1],
    // F
    [1,1,1,1,1,1,1,1,
     1,0,0,0,0,0,0,1,
     1,0,1,1,1,1,1,1,
     1,0,0,0,0,0,0,1,
     1,0,1,1,1,1,1,1,
     1,0,1,1,1,1,1,1,
     1,0,1,1,1,1,1,1,
     1,1,1,1,1,1,1,1],
];

/// Check if the screen's top-left tiles match the expected hex string.
/// Each hex digit occupies an 8x8 tile starting at (digit_index * 8, 0).
fn screen_matches_hex(screen_greyscale: &[u8], expected_hex: &str) -> bool {
    for (digit_idx, ch) in expected_hex.chars().enumerate() {
        let tile_value = match ch {
            '0'..='9' => (ch as u8 - b'0') as usize,
            'A'..='F' => (ch as u8 - b'A' + 10) as usize,
            'a'..='f' => (ch as u8 - b'a' + 10) as usize,
            _ => panic!("Invalid hex char: {ch}"),
        };
        let tile = &HEX_TILES[tile_value];
        let x_off = digit_idx * 8;

        for ty in 0..8 {
            for tx in 0..8 {
                let screen_pixel = screen_greyscale[ty * 160 + x_off + tx];
                // tile: 0 = foreground (0x00), 1 = background (0xFF)
                // Allow ±8 tolerance (Gambatte uses 0xF8F8F8 mask)
                let expected_pixel = if tile[ty * 8 + tx] == 0 { 0x00 } else { 0xFF };
                let diff = (screen_pixel as i16 - expected_pixel as i16).unsigned_abs();
                if diff > 8 {
                    return false;
                }
            }
        }
    }
    true
}

/// Extract the expected hex output from a Gambatte test filename.
/// Pattern: `_out<HEX>` before the file extension.
fn extract_expected_hex(filename: &str) -> &str {
    // Find the last occurrence of "_out" or "_dmg08_out"
    let stem = filename.strip_suffix(".gb").unwrap_or(filename);
    let marker_pos = stem.rfind("_out").expect("no _out in filename");
    &stem[marker_pos + 4..]
}

fn run_gambatte_hex_test(rom_path: &str) {
    let mut gb = common::load_rom(rom_path);
    common::run_frames(&mut gb, FRAMES);

    let screen = common::screen_to_greyscale(gb.screen());
    let filename = rom_path.rsplit('/').next().unwrap();
    let expected_hex = extract_expected_hex(filename);

    assert!(
        screen_matches_hex(&screen, expected_hex),
        "Gambatte hex test {rom_path}: screen does not show expected hex value 0x{expected_hex}"
    );
}

fn run_gambatte_screenshot_test(rom_path: &str, reference_path: &str) {
    let mut gb = common::load_rom(rom_path);
    common::run_frames(&mut gb, FRAMES);

    let actual = common::screen_to_greyscale(gb.screen());
    let expected = common::load_reference_png(reference_path);

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
        "Gambatte screenshot test {rom_path}: {mismatches} pixel mismatches"
    );
}

fn run_gambatte_blank_test(rom_path: &str) {
    let mut gb = common::load_rom(rom_path);
    common::run_frames(&mut gb, FRAMES);

    let screen = common::screen_to_greyscale(gb.screen());
    // Blank screen = all pixels should be background color (0xFF)
    let non_blank = screen.iter().filter(|&&p| p != 0xFF).count();
    assert_eq!(
        non_blank, 0,
        "Gambatte blank test {rom_path}: expected blank screen, got {non_blank} non-white pixels"
    );
}

macro_rules! gambatte_hex_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            run_gambatte_hex_test($path);
        }
    };
}

macro_rules! gambatte_screenshot_test {
    ($name:ident, $rom:literal, $png:literal) => {
        #[test]
        fn $name() {
            run_gambatte_screenshot_test($rom, $png);
        }
    };
}

macro_rules! gambatte_blank_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            run_gambatte_blank_test($path);
        }
    };
}

// ── display_startstate ──────────────────────────────────────────────────

gambatte_hex_test!(
    display_startstate_stat_1,
    "gambatte/display_startstate/stat_1_dmg08_out85.gb"
);
gambatte_hex_test!(
    display_startstate_stat_2,
    "gambatte/display_startstate/stat_2_dmg08_out84.gb"
);

// ── div ─────────────────────────────────────────────────────────────────

gambatte_hex_test!(
    div_start_inc_1,
    "gambatte/div/start_inc_1_dmg08_outAB.gb"
);
gambatte_hex_test!(
    div_start_inc_2,
    "gambatte/div/start_inc_2_dmg08_outAC.gb"
);

// ── miscmstatirq ────────────────────────────────────────────────────────

gambatte_hex_test!(
    miscmstatirq_lycflag_statwirq_1,
    "gambatte/miscmstatirq/lycflag_statwirq_1_dmg08_out2.gb"
);
gambatte_hex_test!(
    miscmstatirq_lycflag_statwirq_2,
    "gambatte/miscmstatirq/lycflag_statwirq_2_dmg08_out2.gb"
);
gambatte_hex_test!(
    miscmstatirq_lycflag_statwirq_3,
    "gambatte/miscmstatirq/lycflag_statwirq_3_dmg08_out2.gb"
);
gambatte_hex_test!(
    miscmstatirq_lycflag_statwirq_4,
    "gambatte/miscmstatirq/lycflag_statwirq_4_dmg08_out0.gb"
);
gambatte_hex_test!(
    miscmstatirq_m0statwirq_1,
    "gambatte/miscmstatirq/m0statwirq_1_dmg08_out2.gb"
);
gambatte_hex_test!(
    miscmstatirq_m0statwirq_2,
    "gambatte/miscmstatirq/m0statwirq_2_dmg08_out0.gb"
);
gambatte_hex_test!(
    miscmstatirq_m0statwirq_3,
    "gambatte/miscmstatirq/m0statwirq_3_dmg08_out0.gb"
);
gambatte_hex_test!(
    miscmstatirq_m0statwirq_4,
    "gambatte/miscmstatirq/m0statwirq_4_dmg08_out2.gb"
);
gambatte_hex_test!(
    miscmstatirq_m0statwirq_scx2_1,
    "gambatte/miscmstatirq/m0statwirq_scx2_1_dmg08_out0.gb"
);
gambatte_hex_test!(
    miscmstatirq_m0statwirq_scx2_2,
    "gambatte/miscmstatirq/m0statwirq_scx2_2_dmg08_out2.gb"
);
gambatte_hex_test!(
    miscmstatirq_m0statwirq_scx3_1,
    "gambatte/miscmstatirq/m0statwirq_scx3_1_dmg08_out0.gb"
);
gambatte_hex_test!(
    miscmstatirq_m0statwirq_scx3_2,
    "gambatte/miscmstatirq/m0statwirq_scx3_2_dmg08_out2.gb"
);
gambatte_hex_test!(
    miscmstatirq_m0statwirq_scx5_1,
    "gambatte/miscmstatirq/m0statwirq_scx5_1_dmg08_out0.gb"
);
gambatte_hex_test!(
    miscmstatirq_m0statwirq_scx5_2,
    "gambatte/miscmstatirq/m0statwirq_scx5_2_dmg08_out2.gb"
);
gambatte_hex_test!(
    miscmstatirq_m1statwirq_1,
    "gambatte/miscmstatirq/m1statwirq_1_dmg08_out3.gb"
);
gambatte_hex_test!(
    miscmstatirq_m1statwirq_2,
    "gambatte/miscmstatirq/m1statwirq_2_dmg08_out3.gb"
);
gambatte_hex_test!(
    miscmstatirq_m1statwirq_3,
    "gambatte/miscmstatirq/m1statwirq_3_dmg08_out2.gb"
);
gambatte_hex_test!(
    miscmstatirq_m1statwirq_4,
    "gambatte/miscmstatirq/m1statwirq_4_dmg08_out0.gb"
);
gambatte_hex_test!(
    miscmstatirq_m2disable,
    "gambatte/miscmstatirq/m2disable_dmg08_cgb_dmg08_out0.gb"
);

// ── sprites — hex output ────────────────────────────────────────────────

gambatte_hex_test!(
    sprites_late_disable_1,
    "gambatte/sprites/late_disable_1_dmg08_out0.gb"
);
gambatte_hex_test!(
    sprites_late_disable_2,
    "gambatte/sprites/late_disable_2_dmg08_out3.gb"
);
gambatte_hex_test!(
    sprites_late_disable_spx18_1,
    "gambatte/sprites/sprite_late_disable_spx18_1_dmg08_out0.gb"
);
gambatte_hex_test!(
    sprites_late_disable_spx18_2,
    "gambatte/sprites/sprite_late_disable_spx18_2_dmg08_out3.gb"
);
gambatte_hex_test!(
    sprites_late_disable_spx19_1,
    "gambatte/sprites/sprite_late_disable_spx19_1_dmg08_out0.gb"
);
gambatte_hex_test!(
    sprites_late_disable_spx19_2,
    "gambatte/sprites/sprite_late_disable_spx19_2_dmg08_out3.gb"
);
gambatte_hex_test!(
    sprites_late_disable_spx1a_1,
    "gambatte/sprites/sprite_late_disable_spx1A_1_dmg08_out0.gb"
);
gambatte_hex_test!(
    sprites_late_disable_spx1a_2,
    "gambatte/sprites/sprite_late_disable_spx1A_2_dmg08_out3.gb"
);
gambatte_hex_test!(
    sprites_late_disable_spx1b_1,
    "gambatte/sprites/sprite_late_disable_spx1B_1_dmg08_out0.gb"
);
gambatte_hex_test!(
    sprites_late_disable_spx1b_2,
    "gambatte/sprites/sprite_late_disable_spx1B_2_dmg08_out3.gb"
);
gambatte_hex_test!(
    sprites_late_enable_spx18_1,
    "gambatte/sprites/sprite_late_enable_spx18_1_dmg08_out3.gb"
);
gambatte_hex_test!(
    sprites_late_enable_spx18_2,
    "gambatte/sprites/sprite_late_enable_spx18_2_dmg08_out0.gb"
);
gambatte_hex_test!(
    sprites_late_enable_spx19_1,
    "gambatte/sprites/sprite_late_enable_spx19_1_dmg08_out3.gb"
);
gambatte_hex_test!(
    sprites_late_enable_spx1a_1,
    "gambatte/sprites/sprite_late_enable_spx1A_1_dmg08_out3.gb"
);
gambatte_hex_test!(
    sprites_late_enable_spx1a_2,
    "gambatte/sprites/sprite_late_enable_spx1A_2_dmg08_out0.gb"
);
gambatte_hex_test!(
    sprites_late_enable_spx1b_1,
    "gambatte/sprites/sprite_late_enable_spx1B_1_dmg08_out3.gb"
);
gambatte_hex_test!(
    sprites_late_enable_spx1b_2,
    "gambatte/sprites/sprite_late_enable_spx1B_2_dmg08_out0.gb"
);
gambatte_hex_test!(
    sprites_late_late_disable_spx18_1,
    "gambatte/sprites/sprite_late_late_disable_spx18_1_dmg08_out0.gb"
);
gambatte_hex_test!(
    sprites_late_late_disable_spx18_2,
    "gambatte/sprites/sprite_late_late_disable_spx18_2_dmg08_out3.gb"
);
gambatte_hex_test!(
    sprites_late_late_disable_spx19_1,
    "gambatte/sprites/sprite_late_late_disable_spx19_1_dmg08_out0.gb"
);
gambatte_hex_test!(
    sprites_late_late_disable_spx19_2,
    "gambatte/sprites/sprite_late_late_disable_spx19_2_dmg08_out3.gb"
);
gambatte_hex_test!(
    sprites_late_late_disable_spx1a_1,
    "gambatte/sprites/sprite_late_late_disable_spx1A_1_dmg08_out0.gb"
);
gambatte_hex_test!(
    sprites_late_late_disable_spx1a_2,
    "gambatte/sprites/sprite_late_late_disable_spx1A_2_dmg08_out3.gb"
);
gambatte_hex_test!(
    sprites_late_late_disable_spx1b_1,
    "gambatte/sprites/sprite_late_late_disable_spx1B_1_dmg08_out0.gb"
);
gambatte_hex_test!(
    sprites_late_late_disable_spx1b_2,
    "gambatte/sprites/sprite_late_late_disable_spx1B_2_dmg08_out3.gb"
);

// ── dmgpalette_during_m3 — screenshot tests ─────────────────────────────

gambatte_screenshot_test!(
    dmgpalette_during_m3_1,
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_1.gb",
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_1_dmg08.png"
);
gambatte_screenshot_test!(
    dmgpalette_during_m3_2,
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_2.gb",
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_2_dmg08.png"
);
gambatte_screenshot_test!(
    dmgpalette_during_m3_3,
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_3.gb",
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_3_dmg08.png"
);
gambatte_screenshot_test!(
    dmgpalette_during_m3_4,
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_4.gb",
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_4_dmg08.png"
);
gambatte_screenshot_test!(
    dmgpalette_during_m3_5,
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_5.gb",
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_5_dmg08.png"
);
gambatte_screenshot_test!(
    dmgpalette_during_m3_scx1_1,
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_scx1_1.gb",
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_scx1_1_dmg08.png"
);
gambatte_screenshot_test!(
    dmgpalette_during_m3_scx1_4,
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_scx1_4.gb",
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_scx1_4_dmg08.png"
);
gambatte_screenshot_test!(
    dmgpalette_during_m3_scx2_1,
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_scx2_1.gb",
    "gambatte/dmgpalette_during_m3/dmgpalette_during_m3_scx2_1_dmg08.png"
);
gambatte_screenshot_test!(
    lycint_dmgpalette_during_m3_1,
    "gambatte/dmgpalette_during_m3/lycint_dmgpalette_during_m3_1.gb",
    "gambatte/dmgpalette_during_m3/lycint_dmgpalette_during_m3_1_dmg08.png"
);
gambatte_screenshot_test!(
    lycint_dmgpalette_during_m3_2,
    "gambatte/dmgpalette_during_m3/lycint_dmgpalette_during_m3_2.gb",
    "gambatte/dmgpalette_during_m3/lycint_dmgpalette_during_m3_2_dmg08.png"
);
gambatte_screenshot_test!(
    lycint_dmgpalette_during_m3_3,
    "gambatte/dmgpalette_during_m3/lycint_dmgpalette_during_m3_3.gb",
    "gambatte/dmgpalette_during_m3/lycint_dmgpalette_during_m3_3_dmg08.png"
);
gambatte_screenshot_test!(
    lycint_dmgpalette_during_m3_4,
    "gambatte/dmgpalette_during_m3/lycint_dmgpalette_during_m3_4.gb",
    "gambatte/dmgpalette_during_m3/lycint_dmgpalette_during_m3_4_dmg08.png"
);
gambatte_screenshot_test!(
    dmgpalette_during_m3_scx3_1,
    "gambatte/dmgpalette_during_m3/scx3/dmgpalette_during_m3_1.gb",
    "gambatte/dmgpalette_during_m3/scx3/dmgpalette_during_m3_1_dmg08.png"
);
gambatte_screenshot_test!(
    dmgpalette_during_m3_scx3_2,
    "gambatte/dmgpalette_during_m3/scx3/dmgpalette_during_m3_2.gb",
    "gambatte/dmgpalette_during_m3/scx3/dmgpalette_during_m3_2_dmg08.png"
);
gambatte_screenshot_test!(
    dmgpalette_during_m3_scx3_3,
    "gambatte/dmgpalette_during_m3/scx3/dmgpalette_during_m3_3.gb",
    "gambatte/dmgpalette_during_m3/scx3/dmgpalette_during_m3_3_dmg08.png"
);
gambatte_screenshot_test!(
    dmgpalette_during_m3_scx3_4,
    "gambatte/dmgpalette_during_m3/scx3/dmgpalette_during_m3_4.gb",
    "gambatte/dmgpalette_during_m3/scx3/dmgpalette_during_m3_4_dmg08.png"
);
gambatte_screenshot_test!(
    dmgpalette_during_m3_scx3_5,
    "gambatte/dmgpalette_during_m3/scx3/dmgpalette_during_m3_5.gb",
    "gambatte/dmgpalette_during_m3/scx3/dmgpalette_during_m3_5_dmg08.png"
);

// ── halt — screenshot tests ─────────────────────────────────────────────

gambatte_screenshot_test!(
    halt_lycint_dmgpalette_during_m3_1,
    "gambatte/halt/lycint_dmgpalette_during_m3_1.gb",
    "gambatte/halt/lycint_dmgpalette_during_m3_1.png"
);
gambatte_screenshot_test!(
    halt_lycint_dmgpalette_during_m3_2,
    "gambatte/halt/lycint_dmgpalette_during_m3_2.gb",
    "gambatte/halt/lycint_dmgpalette_during_m3_2.png"
);
gambatte_screenshot_test!(
    halt_lycint_dmgpalette_during_m3_3,
    "gambatte/halt/lycint_dmgpalette_during_m3_3.gb",
    "gambatte/halt/lycint_dmgpalette_during_m3_3.png"
);
gambatte_screenshot_test!(
    halt_lycint_dmgpalette_during_m3_4,
    "gambatte/halt/lycint_dmgpalette_during_m3_4.gb",
    "gambatte/halt/lycint_dmgpalette_during_m3_4.png"
);

// ── halt — blank screen tests ───────────────────────────────────────────

gambatte_blank_test!(
    halt_ime_noie_nolcdirq_blank,
    "gambatte/halt/ime_noie_nolcdirq_readstat_dmg08_cgb_blank.gb"
);
gambatte_blank_test!(
    halt_noime_noie_nolcdirq_blank,
    "gambatte/halt/noime_noie_nolcdirq_readstat_dmg08_cgb_blank.gb"
);
