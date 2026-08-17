//! The Atari VCS's load-path registration: media recognition, control
//! labels, and the console factory over the crate's seam implementation.

use missingno_core::ports::{PeripheralId, PortId};
use missingno_gamedb::Controller;
use missingno_vcs::debug::{JOYSTICK, KEYPAD, LEFT_PORT, PADDLES, RIGHT_PORT};

use super::{ControlMap, MediaLoad, SystemConsole, TvStandard};

pub use missingno_vcs::debug::{BOARD, OVERDUMP, TV_STANDARD, launch_options};

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

/// The catalogue's word on a cart that carries no header of its own: the
/// standard to decode for, the board the dump sits on, and whether it runs past
/// the silicon. Absent, the core probes and infers.
pub fn create_console(media: MediaLoad) -> Result<Box<dyn SystemConsole>, String> {
    missingno_vcs::debug::create_console(
        media.rom,
        media.fallback_title,
        media
            .launch
            .choice(TV_STANDARD)
            .and_then(TvStandard::from_code),
        media.launch.choice(BOARD),
        media.launch.toggle(OVERDUMP),
    )
    .map_err(|error| error.to_string())
}
