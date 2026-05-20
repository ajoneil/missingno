//! Shared GBC accuracy-test helpers.
//!
//! Re-exports the generic runner helpers from `missingno_gb::test_support`
//! and provides GBC-flavoured ROM loading. ROMs are sourced from
//! `crates/missingno-gb/tests/accuracy/roms/` via `test_support::rom_path`.

use missingno_gb::cartridge::Cartridge;
use missingno_gbc::GameBoyColor;

#[allow(unused_imports)]
pub use missingno_gb::test_support::{
    System, check_mooneye_pass, format_registers, format_wram_dump, is_infinite_loop,
    load_reference_png, rom_path, run_frames, run_until_breakpoint, run_until_infinite_loop,
    run_until_serial_match, run_until_undefined_opcode, screen_to_greyscale,
};

/// Load a ROM into a `GameBoyColor` with no boot ROM (post-boot state).
///
/// No equivalent of `DMG_BOOT_ROM` yet — CGB boot ROM support will land
/// alongside the proper CGB-flag handshake.
pub fn load_rom(relative: &str) -> GameBoyColor {
    let path = rom_path(relative);
    let rom = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("Failed to read ROM {}: {e}", path.display()));
    GameBoyColor::new(Cartridge::new(rom, None), None)
}
