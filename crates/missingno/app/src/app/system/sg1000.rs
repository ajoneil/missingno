//! The Sega SG-1000 family's load-path registration: media recognition,
//! control labels, and the console factory over the crate's machine binding.

use missingno_core::TvStandard;
use missingno_core::launch::TV_STANDARD;

use super::{ControlMap, MediaLoad, SystemConsole};

pub use missingno_sg1000::debug::{is_sg1000_rom, launch_options};

pub const ROM_EXTENSIONS: &[&str] = &["sg"];

/// Both control pads, plus the console's Pause switch.
pub const CONTROLS: ControlMap = ControlMap::new(
    &[],
    missingno_sg1000::debug::PORTS,
    missingno_sg1000::debug::PANEL,
);

/// The catalogue's word on a cartridge that carries no header of its own: the
/// board its silicon sits on and the standard it was cut for. Absent, the image
/// loads as a plain ROM on an NTSC board.
pub fn create_console(media: MediaLoad) -> Result<Box<dyn SystemConsole>, String> {
    let board = missingno_sg1000::debug::board_from_launch(&media.launch)?;
    let standard = media
        .launch
        .choice(TV_STANDARD)
        .and_then(TvStandard::from_name);
    missingno_sg1000::debug::create_console(media.rom, media.fallback_title, board, standard)
}
