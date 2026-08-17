use crate::common;
use crate::common::System;

// Gambatte's testrunner runs each ROM for exactly 1,053,360 T-cycles
// (15 LCD frames at single speed). Tests don't depend on frame events
// so we use a cycle budget — frame-based runners hang on ROMs that
// never enable the LCD.
const TCYCLES: u32 = 1_053_360;

/// Extract the expected hex output from a Gambatte test filename.
/// Pattern: `_out<HEX>` before the file extension.
fn extract_expected_hex(filename: &str) -> &str {
    let stem = filename
        .strip_suffix(".gbc")
        .or_else(|| filename.strip_suffix(".gb"))
        .unwrap_or(filename);
    // Dual marker (_dmg08_cgb04c_out<HEX>): shared output, take it.
    if let Some(pos) = stem.find("_dmg08_cgb04c_out") {
        let after = &stem[pos + "_dmg08_cgb04c_out".len()..];
        let end = after.find('_').unwrap_or(after.len());
        return &after[..end];
    }
    // Otherwise look for DMG-specific marker (handles both
    // `_dmg08_out<HEX>` alone and `_dmg08_out<HEX>_cgb04c_out<HEX2>`
    // double-tagged ROMs).
    let marker = "_dmg08_out";
    let pos = stem
        .find(marker)
        .expect("no _dmg08_out or _dmg08_cgb04c_out marker");
    let after = &stem[pos + marker.len()..];
    let end = after.find('_').unwrap_or(after.len());
    &after[..end]
}

fn run_gambatte_hex_test(rom_path: &str) {
    let mut run = common::load_rom(rom_path);
    common::run_for_tcycles(&mut run, TCYCLES);

    let screen = run.screen_greyscale();
    let filename = rom_path.rsplit('/').next().unwrap();
    let expected_hex = extract_expected_hex(filename);

    if !common::screen_matches_hex(&screen, expected_hex) {
        let shown = common::decode_screen_hex(&screen, expected_hex.len());
        panic!("Gambatte hex test {rom_path}: screen shows 0x{shown}, expected 0x{expected_hex}");
    }
}

fn run_gambatte_screenshot_test(rom_path: &str, reference_path: &str) {
    let mut run = common::load_rom(rom_path);
    common::run_for_tcycles(&mut run, TCYCLES);

    common::assert_screen_matches(
        &format!("Gambatte screenshot test {rom_path}"),
        &run.screen_greyscale(),
        reference_path,
    );
}

/// Extract the DMG-expected audio outcome from a Gambatte test
/// filename. Dual-tagged ROMs (`_dmg08_outaudioN_cgb04c_outaudioM`)
/// resolve to the DMG marker; shared ROMs (`_dmg08_cgb04c_outaudioN`)
/// to the shared one.
fn extract_expected_audio(filename: &str) -> bool {
    let stem = filename
        .strip_suffix(".gbc")
        .or_else(|| filename.strip_suffix(".gb"))
        .unwrap_or(filename);
    if let Some(pos) = stem.find("_dmg08_cgb04c_outaudio") {
        let c = stem.as_bytes()[pos + "_dmg08_cgb04c_outaudio".len()];
        return c == b'1';
    }
    let marker = "_dmg08_outaudio";
    let pos = stem
        .find(marker)
        .expect("no _dmg08_outaudio or _dmg08_cgb04c_outaudio marker");
    stem.as_bytes()[pos + marker.len()] == b'1'
}

/// Gambatte audio test. Filename contains `_outaudio0` (silent
/// expected) or `_outaudio1` (audio expected). Matches gambatte's
/// testrunner convention (testrunner.cpp:263-268): pass if the LAST
/// FRAME's samples are all equal to the first sample of that frame
/// (`_outaudio0`) or NOT all equal (`_outaudio1`). Transient audio earlier in
/// the run is expected on hardware and tolerated. Tolerance 0.005 accounts for
/// APU DC-offset drift.
fn run_gambatte_audio_test(rom_path: &str) {
    let mut run = common::load_rom(rom_path);
    let _ = run.gb.drain_audio_samples();
    common::run_for_tcycles(&mut run, TCYCLES);

    let samples = run.gb.drain_audio_samples();
    let samples_per_frame = samples.len() / 15;
    let last_frame_start = samples.len().saturating_sub(samples_per_frame);
    let last_frame = &samples[last_frame_start..];
    let any_audio = if let Some(&(l0, r0)) = last_frame.first() {
        last_frame
            .iter()
            .any(|&(l, r)| (l - l0).abs() > 0.005 || (r - r0).abs() > 0.005)
    } else {
        false
    };

    let filename = rom_path.rsplit('/').next().unwrap();
    let expect_audio = extract_expected_audio(filename);
    assert_eq!(
        any_audio,
        expect_audio,
        "Gambatte audio test {rom_path}: expected audio={expect_audio}, got audio={any_audio} (samples={})",
        samples.len(),
    );
}

fn run_gambatte_blank_test(rom_path: &str) {
    let mut run = common::load_rom(rom_path);
    common::run_for_tcycles(&mut run, TCYCLES);

    let screen = run.screen_greyscale();
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

macro_rules! gambatte_audio_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            run_gambatte_audio_test($path);
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

gambatte_hex_test!(div_start_inc_1, "gambatte/div/start_inc_1_dmg08_outAB.gb");
gambatte_hex_test!(div_start_inc_2, "gambatte/div/start_inc_2_dmg08_outAC.gb");

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

// `_blank` is not a suffix the upstream gambatte testrunner recognises —
// `testrunner.cpp` dispatches only on `_out` / `dmg08_out` / `dmg08_cgb04c_out`
// and falls back to companion PNGs, none of which exist for these ROMs. Upstream
// therefore silently skips them and never assigns a pass criterion. Both ROMs
// HALT permanently (`IE=0, IF=0` — no interrupt can dispatch or halt-bug),
// and the assertion "screen entirely 0xFF after 15 frames" is incompatible with
// real DMG post-boot state, which leaves the Nintendo logo in VRAM under
// `LCDC=0x91` / `BGP=0xFC` (the BIOS-final value). Ignored pending a
// hardware-verifiable replacement criterion.
// https://github.com/pokemon-speedrunning/gambatte-core/blob/master/test/testrunner.cpp
#[ignore]
#[test]
fn halt_ime_noie_nolcdirq_blank() {
    run_gambatte_blank_test("gambatte/halt/ime_noie_nolcdirq_readstat_dmg08_cgb_blank.gb");
}
#[ignore]
#[test]
fn halt_noime_noie_nolcdirq_blank() {
    run_gambatte_blank_test("gambatte/halt/noime_noie_nolcdirq_readstat_dmg08_cgb_blank.gb");
}

include!("gambatte_shared_tests.rs");
