//! System-agnostic seam vocabulary: the plain data a frontend exchanges with
//! an emulated console — controls, a frame's outcome, the running-status
//! summary — plus the save-state error contract and the behavioural seam
//! traits the frontend drives a console through.

use std::any::Any;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::HighPass;
use crate::cdl::CdlWindow;
use crate::inspect::{MemoryRegion, RegisterGroup, Watch, Watchable};
use crate::isa::InstructionSet;
use crate::symbols::{Symbol, SymbolTable};
use crate::video::{Frame, VideoOut};

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

/// An owned, model-erased per-vblank inspection snapshot the UI renders from
/// while the core runs on the emulation thread. Its [`family_state`] is the
/// family's typed state, downcast back out by that family's own panes.
///
/// [`family_state`]: InspectSnapshot::family_state
pub trait InspectSnapshot: Send {
    fn frame(&self) -> u64;
    fn family_state(&self) -> &dyn Any;
}

/// The model-erased snapshot handed from the emulation thread to the UI.
pub type DebugView = Box<dyn InspectSnapshot>;

/// A running console: everything the plain emulator shell and the emulation
/// thread need from a system.
pub trait SystemConsole: Send {
    /// Emulate up to one frame, with a step budget so an idle display can't
    /// stall the loop.
    fn step_frame(&mut self) -> FrameOutcome;
    fn reset(&mut self);
    /// Apply an input to a family-interpreted control.
    fn set_control(&mut self, control: ControlId, input: ControlInput);
    /// Latching console switches shown as in-play toggles (empty for
    /// families without any).
    fn console_switches(&self) -> &'static [ConsoleSwitch] {
        &[]
    }
    /// Whether this console renders through a user-selectable monochrome
    /// palette (DMG). The play-mode Display panel shows its palette picker
    /// only when true; colour and TV systems return false.
    fn uses_monochrome_palette(&self) -> bool {
        false
    }
    /// Stereo samples at 44.1 kHz — the seam's fixed rate. Families
    /// convert from their native rate on their own side.
    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)>;
    /// The coupling the console's board puts between its audio pads and the
    /// jack. `None` for a family whose board has not been modelled — its
    /// samples reach the device as the chip drove them.
    fn audio_coupling(&self) -> Option<HighPass> {
        None
    }
    fn screen_display(&self) -> Frame;
    /// How this console presents its video: a fixed-size LCD, or a TV raster.
    fn video_out(&self) -> VideoOut;
    /// The game's title for filenames and session records.
    fn game_title(&self) -> String;
    /// Serialized battery-backed save contents, if the media persists any.
    fn battery_save(&self) -> Option<Vec<u8>> {
        None
    }
    /// Wall-clock duration of one emulated frame, for the pacing loop.
    fn frame_interval(&self) -> Duration;
    /// A serialized machine state, if the system has a save-state backend.
    fn save_state(&self) -> Option<Vec<u8>> {
        None
    }
    /// Restore a previously serialized machine state.
    fn load_state(&mut self, _bytes: &[u8]) -> Result<(), StateError> {
        Err(StateError::Unsupported)
    }
    /// Convert to the debugger-backed form. Systems without a debugger
    /// backend hand the console back; callers fall back to plain emulation.
    fn into_debugger(self: Box<Self>) -> Result<Box<dyn SystemDebugger>, Box<dyn SystemConsole>>;
}

/// Why a stepping call returned, and the displayable frame it produced.
pub enum StepOutcome {
    /// Ran to a natural boundary (a completed frame, or a single instruction).
    Completed { frame: Option<Frame> },
    /// Stopped on a PC breakpoint; a frame may still have completed first.
    Breakpoint { frame: Option<Frame> },
    /// Stopped on a watch condition.
    WatchHit(Watch),
    /// Exhausted the step budget without completing a frame or stopping.
    BudgetExhausted,
}

impl StepOutcome {
    /// The frame to display for this stop, if any.
    pub fn into_frame(self) -> Option<Frame> {
        match self {
            StepOutcome::Completed { frame } | StepOutcome::Breakpoint { frame } => frame,
            StepOutcome::WatchHit(_) | StepOutcome::BudgetExhausted => None,
        }
    }
}

/// A console under a debugger: stepping, breakpoints, and inspection.
///
/// Watchpoints, symbols, code/data logging, and trace capture default to
/// absent — a family implements only the backends it has. Breakpoints and
/// peeks cross the seam as `u32`; a core masks them to its own bus width.
pub trait SystemDebugger: Send {
    fn step(&mut self) -> StepOutcome;
    fn step_over(&mut self) -> StepOutcome;
    /// Step until the next frame or a stop.
    fn step_frame(&mut self) -> StepOutcome;
    /// The current screen as it stands, without stepping — for screenshots
    /// taken while paused.
    fn screen_display(&self) -> Frame;
    fn reset(&mut self);
    fn set_control(&mut self, control: ControlId, input: ControlInput);
    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)>;
    /// The coupling the console's board puts between its audio pads and the
    /// jack; `None` for a family whose board has not been modelled.
    fn audio_coupling(&self) -> Option<HighPass> {
        None
    }

    fn set_breakpoint(&mut self, address: u32);
    fn clear_breakpoint(&mut self, address: u32);
    fn breakpoints(&self) -> BTreeSet<u32>;

    /// The register groups this core exposes for the registers view.
    fn register_groups(&self) -> Vec<RegisterGroup> {
        Vec::new()
    }
    /// The CPU-visible address map, named by role.
    fn memory_regions(&self) -> &'static [MemoryRegion] {
        &[]
    }
    /// Side-effect-free read of the CPU address space.
    fn peek(&self, _address: u32) -> u8 {
        0xFF
    }
    /// The address the debugger keys instructions on.
    fn pc(&self) -> u32 {
        0
    }
    /// The core's decode-for-display front end, if it has one.
    fn instruction_set(&self) -> Option<&dyn InstructionSet> {
        None
    }

    /// The watch conditions this core can name.
    fn watchables(&self) -> &'static [Watchable] {
        &[]
    }
    fn add_watch(&mut self, _watch: Watch) {}
    fn remove_watch(&mut self, _watch: &Watch) {}
    fn watches(&self) -> Vec<Watch> {
        Vec::new()
    }
    fn last_watch_hit(&self) -> Option<Watch> {
        None
    }

    /// Labels from the ROM's debug-symbol sidecar, if one was loaded.
    fn symbols(&self) -> Arc<SymbolTable> {
        empty_symbols()
    }
    /// Create a user label at an address; the system decides the bank from
    /// the current mapping.
    fn add_symbol(&mut self, _address: u32, _name: String) {}
    fn remove_symbol(&mut self, _symbol: &Symbol) {}
    /// Code/data-log flags around the current instruction, for the
    /// disassembly's data-byte display.
    fn cdl_window(&self) -> CdlWindow {
        CdlWindow::default()
    }
    /// Load debug sidecars found beside the ROM (the Game Boy's `.sym`
    /// labels and `.cdl` code/data log); families without any do nothing.
    fn load_sidecars(&mut self, _rom_path: &Path) {}
    /// Write updated debug sidecars back beside the ROM.
    fn save_sidecars(&self, _rom_path: &Path) {}
    /// An owned per-vblank snapshot for the UI to render from while running.
    fn snapshot(&self, frame: u64) -> DebugView;
    fn running_status(&self, frame: u64) -> RunningStatus;
    /// The family's typed inspection state, for its own panes to downcast —
    /// the live console while paused.
    fn family_state(&self) -> &dyn Any;

    fn game_title(&self) -> String;
    fn battery_save(&self) -> Option<Vec<u8>> {
        None
    }
    fn frame_interval(&self) -> Duration;
    /// How this console presents its video: a fixed-size LCD, or a TV raster.
    fn video_out(&self) -> VideoOut;
    /// Step one frame while writing an execution trace to `path`; `None` when
    /// the system has no capture backend or capture fails.
    fn capture_trace(&mut self, _path: &Path) -> Option<Frame> {
        None
    }
    /// A serialized machine state, if the system has a save-state backend.
    fn save_state(&self) -> Option<Vec<u8>> {
        None
    }
    /// Restore a previously serialized machine state.
    fn load_state(&mut self, _bytes: &[u8]) -> Result<(), StateError> {
        Err(StateError::Unsupported)
    }

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole>;
}

/// The shared empty table behind the default [`SystemDebugger::symbols`].
fn empty_symbols() -> Arc<SymbolTable> {
    use std::sync::OnceLock;
    static EMPTY: OnceLock<Arc<SymbolTable>> = OnceLock::new();
    EMPTY
        .get_or_init(|| Arc::new(SymbolTable::default()))
        .clone()
}
