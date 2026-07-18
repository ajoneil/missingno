//! The Atari VCS's load-path registration: media recognition, control
//! labels, and the console factory over the crate's seam implementation.

use missingno_vcs::cartridge::CartridgeError;

use super::{SystemConsole, TvStandard};

pub const ROM_EXTENSIONS: &[&str] = &["a26", "bin"];

/// The family's names for the shared control ids, indexed by id.
/// Start/Select work the console switches; both buttons fire.
pub const CONTROL_LABELS: [&str; 8] = [
    "Reset", "Select", "Fire", "Fire", "Up", "Down", "Left", "Right",
];

/// A `.a26` is always ours; a `.bin` only at the family's bare ROM sizes
/// (Game Boy ROMs start at 32 KiB, so the ranges cannot collide).
pub fn is_vcs_rom(path: &std::path::Path, rom: &[u8]) -> bool {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("a26") => true,
        Some("bin") => matches!(rom.len(), 0x800 | 0x1000),
        _ => false,
    }
}

pub fn create_console(
    rom: &[u8],
    title: String,
    tv_standard: Option<TvStandard>,
    cart_type: Option<&str>,
) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    missingno_vcs::debug::create_console(rom, title, tv_standard, cart_type)
}
