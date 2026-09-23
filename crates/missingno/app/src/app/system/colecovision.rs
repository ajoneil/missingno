//! The ColecoVision family's load-path registration: media recognition,
//! control labels, and the console factory over the crate's machine binding.

use missingno_core::TvStandard;
use missingno_core::launch::TV_STANDARD;

use super::{ControlMap, MediaLoad, SystemConsole};

pub use missingno_colecovision::debug::{is_colecovision_rom, launch_options, title_from_rom};

pub const ROM_EXTENSIONS: &[&str] = &["col"];

/// Both hand controllers, plus the console's Reset button.
pub const CONTROLS: ControlMap = ControlMap::new(
    &[],
    missingno_colecovision::debug::PORTS,
    missingno_colecovision::debug::PANEL,
);

/// The standard the catalogue or the user states, and the BIOS the loader
/// resolved out of the firmware folder.
pub fn create_console(media: MediaLoad) -> Result<Box<dyn SystemConsole>, String> {
    let bios = missingno_colecovision::firmware::bios_from_launch(&media.launch)
        .map_err(|(slot, refusal)| format!("{slot}: {refusal}"))?;
    let standard = media
        .launch
        .choice(TV_STANDARD)
        .and_then(TvStandard::from_name);
    let title = title_from_rom(media.rom).unwrap_or(media.fallback_title);
    missingno_colecovision::debug::create_console(media.rom, title, standard, bios)
}
