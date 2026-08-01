//! The Atari VCS's load-path registration: media recognition, control
//! labels, and the console factory over the crate's seam implementation.

use missingno_core::ports::{PeripheralId, PortId};
use missingno_gamedb::Controller;
use missingno_vcs::cartridge::CartridgeError;
use missingno_vcs::debug::{JOYSTICK, KEYPAD, LEFT_PORT, PADDLES, RIGHT_PORT};

use super::{ControlMap, SystemConsole, TvStandard};

pub use missingno_vcs::debug::is_vcs_rom;

pub const ROM_EXTENSIONS: &[&str] = &["a26", "bin"];

/// The console panel and the controllers its two jacks take.
pub const CONTROLS: ControlMap = ControlMap::new(
    &[],
    missingno_vcs::debug::PORTS,
    missingno_vcs::debug::PANEL_CONTROLS,
);

/// What the jacks carry for a game the catalogue describes: key and knob input
/// is inert until the peripheral is plugged. A keypad game wants one in each
/// jack unless it also states the joystick, the arrangement keypad-plus-joystick
/// titles use — stick left, keypad right. Everything else plays on the joysticks
/// a VCS powers on with.
pub fn port_config(controllers: &[Controller]) -> Vec<(PortId, PeripheralId)> {
    let stated = |controller| controllers.contains(&controller);
    if stated(Controller::Keypad) {
        if stated(Controller::Joystick) {
            vec![(LEFT_PORT, JOYSTICK), (RIGHT_PORT, KEYPAD)]
        } else {
            vec![(LEFT_PORT, KEYPAD), (RIGHT_PORT, KEYPAD)]
        }
    } else if stated(Controller::Paddle) {
        vec![(LEFT_PORT, PADDLES)]
    } else {
        Vec::new()
    }
}

pub fn create_console(
    rom: &[u8],
    title: String,
    tv_standard: Option<TvStandard>,
    cart_type: Option<&str>,
    overdump: bool,
) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    missingno_vcs::debug::create_console(rom, title, tv_standard, cart_type, overdump)
}
