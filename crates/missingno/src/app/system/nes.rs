//! The NES / Famicom family's load-path registration: media recognition,
//! control labels, and the console factory over the crate's stepping system.

use missingno_core::stepping::SteppingConsole;
use missingno_nes::cartridge::CartridgeError;
use missingno_nes::console::Nes;
use missingno_nes::debug::NesSystem;

use super::SystemConsole;

pub use missingno_nes::debug::is_nes_rom;

pub const ROM_EXTENSIONS: &[&str] = &["nes"];

/// The family's names for the shared control ids, indexed by id.
pub const CONTROL_LABELS: [&str; 8] = ["Start", "Select", "A", "B", "Up", "Down", "Left", "Right"];

pub fn create_console(rom: &[u8], title: String) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    Ok(Box::new(SteppingConsole::<NesSystem>::new(
        Nes::new(rom)?,
        title,
    )))
}
