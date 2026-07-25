//! The Atari VCS's load-path registration: media recognition, control
//! labels, and the console factory over the crate's seam implementation.

use missingno_vcs::cartridge::CartridgeError;

use super::{ControlMap, SystemConsole, TvStandard};

pub use missingno_vcs::debug::is_vcs_rom;

pub const ROM_EXTENSIONS: &[&str] = &["a26", "bin"];

/// The console panel and the controllers its two jacks take.
pub const CONTROLS: ControlMap = ControlMap::new(
    &[],
    missingno_vcs::debug::PORTS,
    missingno_vcs::debug::PANEL_CONTROLS,
);

pub fn create_console(
    rom: &[u8],
    title: String,
    tv_standard: Option<TvStandard>,
    cart_type: Option<&str>,
) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    missingno_vcs::debug::create_console(rom, title, tv_standard, cart_type)
}
