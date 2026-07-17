//! System-agnostic seam vocabulary: the plain data a frontend exchanges with
//! an emulated console — controls, a frame's outcome, the running-status
//! summary — plus the save-state error contract. The behavioural seam traits
//! stay with the frontend; these are the payloads that cross it.

use crate::video::Frame;

/// A family-interpreted control identifier. Ids 0-7 mirror the Game Boy
/// button order so the existing bindings pipeline translates numerically;
/// analog and family-specific controls take ids from 8 up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ControlId(pub u8);

#[derive(Clone, Copy, Debug)]
pub enum ControlInput {
    Digital(bool),
    /// Normalised 0.0-1.0 (paddle knobs, pots).
    Axis(f32),
}

/// A latching console switch a family exposes for in-play toggling — the
/// VCS's difficulty and colour switches. Unlike the momentary controls on
/// the key-binding path, these hold a position the user flips; toggling one
/// sends its new level through `set_control` as `ControlInput::Digital`.
#[derive(Clone, Copy, Debug)]
pub struct ConsoleSwitch {
    pub control: ControlId,
    pub label: &'static str,
    /// Position names for the two levels, `[low, high]`.
    pub positions: [&'static str; 2],
    /// The power-on level, matching the core's default switch state.
    pub default_high: bool,
}

/// One emulated frame's outcome, as seen by the emu-thread loop.
pub struct FrameOutcome {
    pub display: Option<Frame>,
    pub sram_dirty: bool,
}

/// Live console state published each frame while the debugger runs, so the UI
/// can render its running view without owning the console.
#[derive(Clone, Debug)]
pub struct RunningStatus {
    pub pc: u32,
    pub sp: u32,
    /// The video section's sidebar heading ("PPU", "TIA", ...).
    pub video_label: &'static str,
    /// One-line video position summary in that section.
    pub video_summary: String,
    pub frame: u64,
}

/// Why a save-state operation could not complete.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StateError {
    /// The system has no save-state backend.
    Unsupported,
    /// The state was written for a different ROM.
    IncompatibleRom,
    /// The state data is malformed.
    Corrupt,
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            StateError::Unsupported => "save states are not supported for this system",
            StateError::IncompatibleRom => "save state was written for a different ROM",
            StateError::Corrupt => "save state data is corrupt",
        })
    }
}

impl std::error::Error for StateError {}
