//! The Atari VCS's load-path registration: media recognition, control
//! labels, and the console factory over the crate's seam implementation.

use missingno_vcs::cartridge::CartridgeError;

use super::{SystemConsole, TvStandard};

pub use missingno_vcs::debug::is_vcs_rom;

pub const ROM_EXTENSIONS: &[&str] = &["a26", "bin"];

/// The family's names for the shared control ids, indexed by id.
/// Start/Select work the console switches; both buttons fire.
pub const CONTROL_LABELS: [&str; 8] = [
    "Reset", "Select", "Fire", "Fire", "Up", "Down", "Left", "Right",
];

pub fn create_console(
    rom: &[u8],
    title: String,
    tv_standard: Option<TvStandard>,
    cart_type: Option<&str>,
) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    missingno_vcs::debug::create_console(rom, title, tv_standard, cart_type)
}
