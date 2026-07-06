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
use missingno_gb::joypad::{Button, DirectionalPad};

/// A family-interpreted control identifier. Ids 0-7 mirror the Game Boy
/// button order so the existing bindings pipeline translates numerically;
/// analog and family-specific controls take ids from 8 up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ControlId(pub u8);

#[derive(Clone, Copy, Debug)]
pub enum ControlInput {
    Digital(bool),
    /// Normalised 0.0-1.0 (paddle knobs, pots). Only families with
    /// analog hardware read it, and those are feature-gated today.
    Axis(#[cfg_attr(not(feature = "vcs"), allow(dead_code))] f32),
}

/// The numeric convention the bindings pipeline maps buttons through.
pub fn control_for_button(button: Button) -> ControlId {
    ControlId(match button {
        Button::Start => 0,
        Button::Select => 1,
        Button::A => 2,
        Button::B => 3,
        Button::DirectionalPad(DirectionalPad::Up) => 4,
        Button::DirectionalPad(DirectionalPad::Down) => 5,
        Button::DirectionalPad(DirectionalPad::Left) => 6,
        Button::DirectionalPad(DirectionalPad::Right) => 7,
    })
}

use std::sync::Arc;

use crate::app::debugger::inspect::{DebugView, Inspection};
use crate::app::debugger::panes;
use crate::app::emu_thread::RunningStatus;
use crate::app::library::activity::FrameCapture;
use crate::app::screen::ScreenDisplay;

pub mod gb;
#[cfg(feature = "nes")]
pub mod nes;
#[cfg(feature = "sms")]
pub mod sms;
#[cfg(any(feature = "nes", feature = "sms"))]
pub mod stepping;
#[cfg(feature = "vcs")]
pub mod vcs;

/// Build a console from ROM bytes and a display title; `None` when the
/// media fails to parse (the loader falls through to the next family).
pub type CreateConsole = fn(&[u8], String) -> Option<Box<dyn SystemConsole>>;

/// A family's registration on the load path: how its media is recognised in
/// file dialogs, library scans, and ROM loads, and how a console is built.
pub struct FamilyDescriptor {
    /// Platform display name; also the file-dialog filter label.
    pub platform_name: &'static str,
    pub extensions: &'static [&'static str],
    /// Whether path and contents identify this family's media.
    pub is_rom: fn(&Path, &[u8]) -> bool,
    pub create_console: CreateConsole,
}

/// Every registered family except the Game Boy, which stays the loader's
/// fallback (its media needs battery saves, boot ROMs, and the link port).
pub static FAMILIES: &[FamilyDescriptor] = &[
    #[cfg(feature = "vcs")]
    FamilyDescriptor {
        platform_name: vcs::PLATFORM_NAME,
        extensions: vcs::ROM_EXTENSIONS,
        is_rom: vcs::is_vcs_rom,
        create_console: |rom, title| vcs::create_console(rom, title).ok(),
    },
    #[cfg(feature = "sms")]
    FamilyDescriptor {
        platform_name: sms::PLATFORM_NAME,
        extensions: sms::ROM_EXTENSIONS,
        is_rom: |path, _| sms::is_sms_rom(path),
        create_console: |rom, title| sms::create_console(rom, title).ok(),
    },
    #[cfg(feature = "nes")]
    FamilyDescriptor {
        platform_name: nes::PLATFORM_NAME,
        extensions: nes::ROM_EXTENSIONS,
        is_rom: |_, rom| nes::is_nes_rom(rom),
        create_console: |rom, title| nes::create_console(rom, title).ok(),
    },
];

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
    /// Apply an input to a family-interpreted control.
    fn set_control(&mut self, control: ControlId, input: ControlInput);
    /// Stereo samples at 44.1 kHz — the seam's fixed rate. Families
    /// convert from their native rate on their own side.
    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)>;
    fn screen_display(&self) -> ScreenDisplay;
    fn capture_frame(&self, use_sgb_colors: bool, palette_name: &str) -> FrameCapture;
    /// The game's title for filenames and session records.
    fn game_title(&self) -> String;
    /// Serialized battery-backed save contents, if the media persists any.
    fn battery_save(&self) -> Option<Vec<u8>> {
        None
    }
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
///
/// Watchpoints, symbols, code/data logging, and trace capture default to
/// absent — a family implements only the backends it has.
pub trait SystemDebugger: Send {
    fn step(&mut self) -> Option<ScreenDisplay>;
    fn step_over(&mut self) -> Option<ScreenDisplay>;
    /// Step until the next frame or breakpoint. The flag reports a breakpoint
    /// stop.
    fn step_frame(&mut self) -> (Option<ScreenDisplay>, bool);
    fn reset(&mut self);
    fn set_control(&mut self, control: ControlId, input: ControlInput);
    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)>;

    fn set_breakpoint(&mut self, address: u16);
    fn clear_breakpoint(&mut self, address: u16);
    fn breakpoints(&self) -> &BTreeSet<u16>;
    fn add_watchpoint(&mut self, _condition: WatchCondition) {}
    fn remove_watchpoint(&mut self, _condition: &WatchCondition) {}
    fn watchpoints(&self) -> &[WatchCondition] {
        &[]
    }
    fn last_watchpoint_hit(&self) -> Option<WatchCondition> {
        None
    }

    /// The live inspection surface the debugger panes render from while paused.
    fn inspect(&self) -> &dyn Inspection;
    /// The family's debugger pane set and layout identity.
    fn pane_family(&self) -> &'static panes::Family {
        &panes::GB_FAMILY
    }
    /// Labels from the ROM's debug-symbol sidecar, if one was loaded.
    fn symbols(&self) -> Arc<SymbolTable> {
        empty_symbols()
    }
    fn set_symbols(&mut self, _symbols: SymbolTable) {}
    /// Create a user label at an address; the system decides the bank from
    /// the current mapping.
    fn add_symbol(&mut self, _address: u16, _name: String) {}
    fn remove_symbol(&mut self, _symbol: &Symbol) {}
    fn save_symbols(&self, _path: &Path) {}
    /// Code/data-log flags around the current instruction, for the
    /// disassembly's data-byte display.
    fn cdl_window(&self) -> CdlWindow {
        CdlWindow::default()
    }
    fn load_cdl(&mut self, _path: &Path) {}
    fn save_cdl(&self, _path: &Path) {}
    /// An owned per-vblank snapshot for the UI to render from while running.
    fn snapshot(&self, frame: u64) -> DebugView;
    fn running_status(&self, frame: u64) -> RunningStatus;

    fn game_title(&self) -> String;
    fn battery_save(&self) -> Option<Vec<u8>> {
        None
    }
    fn frame_interval(&self) -> Duration;
    fn capture_frame(&self, use_sgb_colors: bool, palette_name: &str) -> FrameCapture;
    /// Step one frame while writing an execution trace to `path`; `None` when
    /// the system has no capture backend or capture fails.
    fn capture_trace(&mut self, _path: &Path) -> Option<ScreenDisplay> {
        None
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
