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

pub use controls::{
    JOYSTICK, KEYPAD, LEFT_PORT, PADDLES, PANEL_CONTROLS, PORTS, RIGHT_PORT, UNPLUGGED,
};
pub use inspect::VcsInspectState;
pub use machine::{VcsSnapshot, VcsSystem, create_console};
pub use sections::vcs_sidebar_sections;

/// A `.a26` is always ours; a `.bin` only at the family's bare ROM sizes
/// (Game Boy ROMs start at 32 KiB, so the ranges cannot collide) or at a
/// Supercharger container's, whose 8448-byte unit no other family shares.
pub fn is_vcs_rom(path: &std::path::Path, rom: &[u8]) -> bool {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("a26") => true,
        Some("bin") => {
            matches!(rom.len(), 0x800 | 0x1000) || crate::cartridge::ar::is_container(rom.len())
        }
        _ => false,
    }
}
