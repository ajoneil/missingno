//! The Atari VCS's machine binding and its debugger inspection state.
//!
//! The seam's wrappers drive the hooks in [`machine`]; frame assembly, controls,
//! and one inspection state — serving both the paused view (refreshed after
//! every step) and the per-frame snapshot the running view renders from — sit
//! beside them.

mod controls;
mod frame;
mod inspect;
mod machine;
mod probe;
mod sections;

pub use controls::{JOYSTICK, KEYPAD, LEFT_PORT, PADDLES, PANEL_CONTROLS, PORTS, RIGHT_PORT};
pub use machine::{
    BOARD, OVERDUMP, TV_STANDARD, board_from_launch, create_console, launch_options,
};
pub use sections::vcs_sidebar_sections;

/// A `.a26` is ours, and nothing else is: `.bin` is a generic dump extension no
/// core may claim, so such a file reaches this core by a database match or an
/// explicit system selection — both outside this predicate.
pub fn is_vcs_rom(path: &std::path::Path, _rom: &[u8]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("a26"))
}
