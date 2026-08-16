//! The Sega SG-1000 family's load-path registration: media recognition,
//! control labels, and the console factory over the crate's machine binding.

use missingno_sg1000::cartridge::CartridgeError;

use super::{ControlMap, SystemConsole};

pub use missingno_sg1000::debug::is_sg1000_rom;

pub const ROM_EXTENSIONS: &[&str] = &["sg"];

/// Both control pads, plus the console's Pause switch.
pub const CONTROLS: ControlMap = ControlMap::new(
    &[],
    missingno_sg1000::debug::PORTS,
    missingno_sg1000::debug::PANEL,
);

pub fn create_console(rom: &[u8], title: String) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    missingno_sg1000::debug::create_console(rom, title)
}
