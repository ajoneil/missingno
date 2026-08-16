//! The Sega Master System family's load-path registration: media
//! recognition, control labels, and the console factory over the crate's
//! machine binding.

use missingno_core::machine::MachineConsole;
use missingno_sms::cartridge::CartridgeError;
use missingno_sms::console::Sms;
use missingno_sms::debug::SmsSystem;

use super::{ControlMap, SystemConsole};

pub use missingno_sms::debug::is_sms_rom;

pub const ROM_EXTENSIONS: &[&str] = &["sms"];

/// The control pad in the first jack, plus the console's Pause button.
pub const CONTROLS: ControlMap = ControlMap::new(
    &[],
    missingno_sms::debug::PORTS,
    missingno_sms::debug::PANEL,
);

pub fn create_console(rom: &[u8], title: String) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    Ok(Box::new(MachineConsole::<SmsSystem>::new(
        Sms::new(rom)?,
        title,
    )))
}
