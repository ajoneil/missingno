//! The seam between the system-agnostic app shell and an emulated system.
//!
//! The app (library, emulator shell, emu thread, debugger UI) drives a console
//! through these object-safe traits; each system family implements them once,
//! in its own submodule, and registers in its factory. Adding a system means
//! adding a submodule — not extending parallel dispatch enums.

use std::collections::BTreeSet;
use std::path::Path;

use missingno_gb::{cartridge::Cartridge, joypad::Button, serial_transfer::SerialLink};

use crate::app::debugger::inspect::{DebugView, InspectSource};
use crate::app::emu_thread::RunningStatus;
use crate::app::library::activity::FrameCapture;
use crate::app::screen::ScreenDisplay;

pub mod gb;

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
    fn cartridge(&self) -> &Cartridge;
    fn set_link(&mut self, link: Box<dyn SerialLink>);
    fn into_debugger(self: Box<Self>) -> Box<dyn SystemDebugger>;
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

    /// The live inspection surface the debugger panes render from while paused.
    fn inspect(&self) -> &dyn InspectSource;
    /// An owned per-vblank snapshot for the UI to render from while running.
    fn snapshot(&self, frame: u64) -> DebugView;
    fn running_status(&self, frame: u64) -> RunningStatus;

    fn cartridge(&self) -> &Cartridge;
    fn capture_frame(&self, use_sgb_colors: bool, palette_name: &str) -> FrameCapture;
    /// Step one frame while writing an execution trace to `path`; `None` on
    /// capture failure.
    fn capture_trace(&mut self, path: &Path) -> Option<ScreenDisplay>;

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole>;
}
