//! The Sega Master System family's load-path registration: media
//! recognition, control labels, and the console factory over the crate's
//! stepping system.

use missingno_core::stepping::SteppingConsole;
use missingno_sms::cartridge::CartridgeError;
use missingno_sms::console::Sms;
use missingno_sms::debug::SmsSystem;

use super::SystemConsole;

pub use missingno_sms::debug::is_sms_rom;

pub const ROM_EXTENSIONS: &[&str] = &["sms"];

/// The family's names for the shared control ids, indexed by id.
/// Start works the console Pause button; Select has no SMS reading.
pub const CONTROL_LABELS: [&str; 8] = [
    "Pause", "", "Button 1", "Button 2", "Up", "Down", "Left", "Right",
];

pub fn create_console(rom: &[u8], title: String) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    Ok(Box::new(SteppingConsole::<SmsSystem>::new(
        Sms::new(rom)?,
        title,
    )))
}
