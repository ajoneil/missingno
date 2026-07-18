//! The Sega Master System family's load-path registration: media
//! recognition, control labels, and the console factory over the crate's
//! stepping system.

use missingno_core::stepping::SteppingConsole;
use missingno_sms::cartridge::CartridgeError;
use missingno_sms::console::Sms;
use missingno_sms::debug::SmsSystem;

use super::SystemConsole;

pub const ROM_EXTENSIONS: &[&str] = &["sms"];

/// The family's names for the shared control ids, indexed by id.
/// Start works the console Pause button; Select has no SMS reading.
pub const CONTROL_LABELS: [&str; 8] = [
    "Pause", "", "Button 1", "Button 2", "Up", "Down", "Left", "Right",
];

pub fn is_sms_rom(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("sms"))
}

pub fn create_console(rom: &[u8], title: String) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    Ok(Box::new(SteppingConsole::<SmsSystem>::new(
        Sms::new(rom)?,
        title,
    )))
}
