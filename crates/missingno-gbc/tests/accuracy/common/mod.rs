//! Shared GBC accuracy-test helpers.
//!
//! ROM resolution:
//! - [`load_rom`] / [`rom_path`] resolve relative to
//!   `crates/missingno-gb/tests/accuracy/roms/` — for ROMs that exist on
//!   both DMG and CGB (the gb crate is the canonical source for shared
//!   ROMs to avoid duplication).
//! - [`load_cgb_rom`] / [`cgb_rom_path`] resolve relative to
//!   `crates/missingno-gbc/tests/accuracy/roms/` — for ROMs that target
//!   only CGB hardware (`cgb-acid2`, `cgb-acid-hell`, `rtc3test`, etc.).

use std::path::{Path, PathBuf};

use missingno_gb::cartridge::Cartridge;
use missingno_gbc::GameBoyColor;

#[allow(unused_imports)]
pub use missingno_gb::test_support::{
    System, check_mooneye_pass, format_registers, format_wram_dump, is_infinite_loop,
    load_reference_png, rom_path, run_for_tcycles, run_frames, run_until_breakpoint,
    run_until_infinite_loop, run_until_infinite_loop_no_lcd, run_until_serial_match,
    run_until_undefined_opcode, screen_to_greyscale,
};

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
    GameBoyColor::new(Cartridge::new(rom, None), None)
}

/// Load a CGB-only ROM from the gbc crate's own roms dir.
pub fn load_cgb_rom(relative: &str) -> GameBoyColor {
    let path = cgb_rom_path(relative);
    let rom = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("Failed to read ROM {}: {e}", path.display()));
    GameBoyColor::new(Cartridge::new(rom, None), None)
}

/// Load a reference PNG from the gbc crate's own roms dir.
pub fn load_cgb_reference_png(relative: &str) -> Vec<u8> {
    let path = cgb_rom_path(relative);
    let file = std::fs::File::open(&path)
        .unwrap_or_else(|e| panic!("Failed to open reference image {}: {e}", path.display()));
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();

    let width = info.width as usize;
    let height = info.height as usize;
    let stride = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => panic!("Unsupported PNG color type: {other:?}"),
    };
    (0..width * height).map(|i| buf[i * stride]).collect()
}
