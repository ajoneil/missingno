//! The Sega SG-1000 family's load-path registration: media recognition,
//! control labels, and the console factory over the crate's machine binding.

use missingno_sg1000::cartridge::CartType;

use super::{ControlMap, MediaLoad, SystemConsole};

pub use missingno_sg1000::debug::{BOARD, is_sg1000_rom, launch_options};

pub const ROM_EXTENSIONS: &[&str] = &["sg"];

/// Both control pads, plus the console's Pause switch.
pub const CONTROLS: ControlMap = ControlMap::new(
    &[],
    missingno_sg1000::debug::PORTS,
    missingno_sg1000::debug::PANEL,
);

/// The catalogue's word on a cartridge that carries no header of its own: the
/// board its silicon sits on. Absent, the image loads as a plain ROM.
pub fn create_console(media: MediaLoad) -> Result<Box<dyn SystemConsole>, String> {
    missingno_sg1000::debug::create_console(
        media.rom,
        media.fallback_title,
        media.launch.choice(BOARD).and_then(CartType::from_code),
    )
    .map_err(|error| error.to_string())
}
