//! The Atari VCS's implementation of the system seam and its debugger
//! inspection state.
//!
//! Two halves of the seam — a plain console driven frame by frame, and the same
//! console under its debugging backend — share the frame assembly, controls,
//! save-state glue, and one inspection state that serves both the paused view
//! (refreshed after every step) and the per-frame snapshot the running view
//! renders from.

mod console;
mod controls;
mod debugger_seam;
mod frame;
mod inspect;
mod probe;
mod save_state;
mod sections;

pub use console::create_console;
pub use controls::{CONSOLE_SWITCHES, PADDLE_CONTROL};
pub use debugger_seam::VcsSnapshot;
pub use inspect::VcsInspectState;
pub use sections::vcs_sidebar_sections;

/// A `.a26` is always ours; a `.bin` only at the family's bare ROM sizes
/// (Game Boy ROMs start at 32 KiB, so the ranges cannot collide).
pub fn is_vcs_rom(path: &std::path::Path, rom: &[u8]) -> bool {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("a26") => true,
        Some("bin") => matches!(rom.len(), 0x800 | 0x1000),
        _ => false,
    }
}
