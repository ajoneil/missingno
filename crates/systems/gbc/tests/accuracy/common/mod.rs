//! Shared GBC accuracy-test helpers.
//!
//! ROM resolution:
//! - [`load_rom`] / [`rom_path`] resolve relative to
//!   `crates/systems/gb/tests/accuracy/roms/` — for ROMs that exist on
//!   both DMG and CGB (the gb crate is the canonical source for shared
//!   ROMs to avoid duplication).
//! - [`load_cgb_rom`] / [`cgb_rom_path`] resolve relative to
//!   `crates/systems/gbc/tests/accuracy/roms/` — for ROMs that target
//!   only CGB hardware (`cgb-acid2`, `cgb-acid-hell`, `rtc3test`, etc.).

use std::path::{Path, PathBuf};

use missingno_gb::cartridge::Cartridge;
use missingno_gb::memory::BootRom;
use missingno_gbc::GameBoyColor;

#[allow(unused_imports)]
pub use missingno_gb::test_support::{
    System, check_mooneye_pass, decode_screen_hex, format_registers, format_wram_dump,
    is_infinite_loop, rom_path, run_boot_rom, run_for_tcycles, run_frames, run_until_breakpoint,
    run_until_infinite_loop, run_until_infinite_loop_no_lcd, run_until_serial_match,
    run_until_undefined_opcode, screen_matches_hex, screen_to_greyscale,
};
pub use missingno_test_support::compare::{assert_pixels_match, debug_value, hex_byte};
use missingno_test_support::reference::ReferencePng;

/// Try to load the CGB boot ROM (2304 bytes) from the path in `CGB_BOOT_ROM`.
/// Returns None if unset or unreadable. Proprietary — not distributed.
pub fn try_load_cgb_boot_rom() -> Option<BootRom> {
    let path = std::env::var("CGB_BOOT_ROM").ok()?;
    let data = std::fs::read(&path).ok()?;
    let boxed: Box<[u8; 0x900]> = data.into_boxed_slice().try_into().ok()?;
    Some(BootRom::Cgb(boxed))
}

/// Build a `GameBoyColor`, loading the CGB boot ROM from `CGB_BOOT_ROM` if
/// set (and driving it to the 0x0100 cartridge handoff). With the env unset
/// the boot ROM is `None` and the core uses its skip-boot post-boot state.
fn new_cgb(rom: Vec<u8>) -> GameBoyColor {
    let mut gbc = GameBoyColor::new(Cartridge::new(rom, None), try_load_cgb_boot_rom());
    run_boot_rom(&mut gbc);
    gbc
}

/// Resolve a path relative to `missingno-gbc/tests/accuracy/roms/`.
/// Use this for CGB-only test ROMs that live in the gbc crate.
pub fn cgb_rom_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/accuracy/roms")
        .join(relative)
}

/// Load a shared DMG+CGB-compatible ROM from the gb crate's roms dir.
pub fn load_rom(relative: &str) -> GameBoyColor {
    let path = rom_path(relative);
    let rom = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("Failed to read ROM {}: {e}", path.display()));
    new_cgb(rom)
}

/// Load a CGB-only ROM from the gbc crate's own roms dir.
pub fn load_cgb_rom(relative: &str) -> GameBoyColor {
    let path = cgb_rom_path(relative);
    let rom = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("Failed to read ROM {}: {e}", path.display()));
    new_cgb(rom)
}

#[cfg(feature = "morepork")]
pub use missingno_gb::test_support::TestRun;

/// Wrap a CGB-only ROM in a [`TestRun`] for execution-trace capture. With the
/// `morepork` feature and `MOREPORK_PROFILE` set, the run writes a `.morepork`
/// under `receipts/traces/`. Mirrors the gb crate's traced `load_rom`.
#[cfg(feature = "morepork")]
pub fn load_cgb_rom_traced(relative: &str) -> TestRun<missingno_gbc::Cgb> {
    let path = cgb_rom_path(relative);
    let rom = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("Failed to read ROM {}: {e}", path.display()));
    TestRun::new(new_cgb(rom), relative, "CGB-C")
}

/// Load a reference PNG from the gbc crate's own roms dir as one shade byte
/// per pixel.
pub fn load_cgb_reference_png(relative: &str) -> Vec<u8> {
    ReferencePng::load(&cgb_rom_path(relative)).greyscale()
}

/// Load a reference PNG from the gb crate's shared roms dir as one shade byte
/// per pixel.
pub fn load_reference_png(relative: &str) -> Vec<u8> {
    ReferencePng::load(&rom_path(relative)).greyscale()
}

/// The RGB analogue of [`load_cgb_reference_png`], for the colourised
/// CGB-compat references where the red channel alone is insufficient.
pub fn load_cgb_reference_png_rgb(relative: &str) -> Vec<[u8; 3]> {
    ReferencePng::load(&cgb_rom_path(relative)).rgb()
}

/// The RGB analogue of [`load_reference_png`], from the shared roms dir.
pub fn load_reference_png_rgb(relative: &str) -> Vec<[u8; 3]> {
    ReferencePng::load(&rom_path(relative)).rgb()
}

/// Re-cut a screen's flat RGB888 bytes as one triple per pixel, the currency
/// the colour references compare in.
pub fn rgb_pixels(bytes: &[u8]) -> Vec<[u8; 3]> {
    bytes.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect()
}
