//! The seam between the system-agnostic app shell and an emulated system.
//!
//! The app (library, emulator shell, emu thread, debugger UI) drives a console
//! through these object-safe traits; each system family implements them once,
//! in its own submodule, and registers in its factory. Adding a system means
//! adding a submodule — not extending parallel dispatch enums.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use missingno_gb::debugger::{
    WatchCondition,
    cdl::CdlWindow,
    symbols::{Symbol, SymbolTable},
};
use missingno_gb::{joypad::Button, serial_transfer::SerialLink};

use std::sync::Arc;

use crate::app::debugger::inspect::{DebugView, Inspection};
use crate::app::debugger::panes;
use crate::app::emu_thread::RunningStatus;
use crate::app::library::activity::FrameCapture;
use crate::app::screen::ScreenDisplay;

pub mod gb;
#[cfg(feature = "vcs")]
pub mod vcs;

/// One emulated frame's outcome, as seen by the emu-thread loop.
pub struct FrameOutcome {
    pub display: Option<ScreenDisplay>,
    pub sram_dirty: bool,
}

/// A running console: everything the plain emulator shell and the emulation
/// thread need from a system.
pub trait SystemConsole: Send {
    /// Emulate up to one frame, with a step budget so an idle display can't
    /// stall the loop.
    fn step_frame(&mut self) -> FrameOutcome;
    fn reset(&mut self);
    fn press_button(&mut self, button: Button);
    fn release_button(&mut self, button: Button);
    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)>;
    fn screen_display(&self) -> ScreenDisplay;
    fn capture_frame(&self, use_sgb_colors: bool, palette_name: &str) -> FrameCapture;
    /// The game's title for filenames and session records.
    fn game_title(&self) -> String;
    /// Serialized battery-backed save contents, if the media persists any.
    fn battery_save(&self) -> Option<Vec<u8>>;
    fn set_link(&mut self, link: Box<dyn SerialLink>);
    /// Wall-clock duration of one emulated frame, for the pacing loop.
    fn frame_interval(&self) -> Duration;
    /// Convert to the debugger-backed form. Systems without a debugger
    /// backend hand the console back; callers fall back to plain emulation.
    fn into_debugger(self: Box<Self>) -> Result<Box<dyn SystemDebugger>, Box<dyn SystemConsole>>;
}

/// A console under a debugger: stepping, breakpoints, and inspection.
///
/// Step results come back as ready-to-show [`ScreenDisplay`]s — the system
/// decides what a step with no completed frame looks like (e.g. an LCD-off
/// screen still displays).
pub trait SystemDebugger: Send {
    fn step(&mut self) -> Option<ScreenDisplay>;
    fn step_over(&mut self) -> Option<ScreenDisplay>;
    /// Step until the next frame or breakpoint. The flag reports a breakpoint
    /// stop.
    fn step_frame(&mut self) -> (Option<ScreenDisplay>, bool);
    fn reset(&mut self);
    fn press_button(&mut self, button: Button);
    fn release_button(&mut self, button: Button);
    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)>;

    fn set_breakpoint(&mut self, address: u16);
    fn clear_breakpoint(&mut self, address: u16);
    fn breakpoints(&self) -> &BTreeSet<u16>;
    fn add_watchpoint(&mut self, condition: WatchCondition);
    fn remove_watchpoint(&mut self, condition: &WatchCondition);
    fn watchpoints(&self) -> &[WatchCondition];
    fn last_watchpoint_hit(&self) -> Option<WatchCondition>;

    /// The live inspection surface the debugger panes render from while paused.
    fn inspect(&self) -> &dyn Inspection;
    /// The family's debugger pane set and layout identity.
    fn pane_family(&self) -> &'static panes::Family {
        &panes::GB_FAMILY
    }
    /// Labels from the ROM's debug-symbol sidecar, if one was loaded.
    fn symbols(&self) -> Arc<SymbolTable>;
    fn set_symbols(&mut self, symbols: SymbolTable);
    /// Create a user label at an address; the system decides the bank from
    /// the current mapping.
    fn add_symbol(&mut self, address: u16, name: String);
    fn remove_symbol(&mut self, symbol: &Symbol);
    fn save_symbols(&self, path: &Path);
    /// Code/data-log flags around the current instruction, for the
    /// disassembly's data-byte display.
    fn cdl_window(&self) -> CdlWindow;
    fn load_cdl(&mut self, path: &Path);
    fn save_cdl(&self, path: &Path);
    /// An owned per-vblank snapshot for the UI to render from while running.
    fn snapshot(&self, frame: u64) -> DebugView;
    fn running_status(&self, frame: u64) -> RunningStatus;

    fn game_title(&self) -> String;
    fn battery_save(&self) -> Option<Vec<u8>>;
    fn frame_interval(&self) -> Duration;
    fn capture_frame(&self, use_sgb_colors: bool, palette_name: &str) -> FrameCapture;
    /// Step one frame while writing an execution trace to `path`; `None` on
    /// capture failure.
    fn capture_trace(&mut self, path: &Path) -> Option<ScreenDisplay>;

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole>;
}
