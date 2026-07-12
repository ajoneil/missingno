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

use std::sync::Arc;

use crate::app::debugger::inspect::{DebugView, Inspection};
use crate::app::debugger::panes;
use crate::app::emu_thread::RunningStatus;
use crate::app::library::activity::{CaptureOptions, FrameCapture};
use crate::app::screen::ScreenDisplay;

pub mod gb;
#[cfg(feature = "nes")]
pub mod nes;
#[cfg(feature = "sms")]
pub mod sms;
#[cfg(any(feature = "nes", feature = "sms"))]
pub mod stepping;
pub mod vcs;

/// The platforms the frontend knows, one per family descriptor. The
/// canonical platform identity for library metadata: external sources'
/// platform strings are mapped into it, and display always goes through
/// [`Platform::name`]. Variants are never cfg-gated — a library entry
/// written by a fuller build must still parse.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Platform {
    GameBoy,
    GameBoyColor,
    AtariVcs,
    MasterSystem,
    Nes,
}

impl Platform {
    /// Display name; also the file-dialog filter label.
    pub fn name(self) -> &'static str {
        match self {
            Platform::GameBoy => "Nintendo Game Boy",
            Platform::GameBoyColor => "Nintendo Game Boy Color",
            Platform::AtariVcs => "Atari VCS",
            Platform::MasterSystem => "Sega Master System",
            Platform::Nes => "Nintendo Entertainment System",
        }
    }

    /// Short name for compact UI (bindings rows, badges).
    pub fn short_name(self) -> &'static str {
        match self {
            Platform::GameBoy => "GB",
            Platform::GameBoyColor => "GBC",
            Platform::AtariVcs => "VCS",
            Platform::MasterSystem => "SMS",
            Platform::Nes => "NES",
        }
    }

    /// Best-effort mapping from an external platform description — a
    /// Hasheous platform name, or the string an older library entry stored.
    pub fn from_description(text: &str) -> Option<Platform> {
        let text = text.to_ascii_lowercase();
        if text.contains("game boy color") {
            Some(Platform::GameBoyColor)
        } else if text.contains("game boy") && !text.contains("advance") {
            Some(Platform::GameBoy)
        } else if text.contains("2600") || text.contains("atari vcs") {
            Some(Platform::AtariVcs)
        } else if text.contains("master system") {
            Some(Platform::MasterSystem)
        } else if text.contains("nintendo entertainment system") || text.contains("famicom") {
            Some(Platform::Nes)
        } else {
            None
        }
    }
}

/// Everything the loader hands a family's console factory. The fields are
/// family-agnostic except the two Game Boy peripheral ones, quarantined here
/// under the same rule as the GB types on the seam traits: generalize when a
/// second family grows an equivalent, not before.
pub struct MediaLoad<'a> {
    /// Soft-patched ROM contents.
    pub rom: &'a [u8],
    /// Display-title fallback (the file stem); families whose media carries
    /// a header title ignore it.
    pub fallback_title: String,
    /// Battery-save contents to restore, if the library holds any.
    pub save_data: Option<Vec<u8>>,
    /// The game's library folder, for peripherals that write artifacts.
    pub game_dir: &'a Path,
    /// Boot ROM supplied on the CLI; the Game Boy family attaches it.
    pub boot_rom: Option<missingno_gb::BootRom>,
    /// Link-cable connection, borrowed mutably so only the family that owns
    /// the concept takes it.
    pub serial_link: &'a mut Option<Box<dyn missingno_gb::serial_transfer::SerialLink>>,
}

/// Build a console from loaded media; `None` when the media fails to parse.
pub type CreateConsole = fn(MediaLoad) -> Option<Box<dyn SystemConsole>>;

/// A gbtrace capture request, dispatched through the family table by the
/// `trace` subcommand.
pub struct TraceRequest<'a> {
    pub rom: &'a [u8],
    /// For the Game Boy family's `.sav` sidecar lookup.
    pub rom_path: &'a Path,
    pub profile: &'a missingno_gb::trace::Profile,
    pub output: &'a Path,
    pub cycles: u64,
    pub boot_rom: Option<missingno_gb::BootRom>,
}

/// A family's registration on the load path: how its media is recognised in
/// file dialogs, library scans, and ROM loads, and how a console is built.
pub struct FamilyDescriptor {
    pub platform: Platform,
    pub extensions: &'static [&'static str],
    /// The family's names for the shared control ids, indexed by id;
    /// empty string for ids the family ignores.
    pub control_labels: &'static [&'static str; 8],
    /// Whether path and contents identify this family's media. Predicates
    /// across the table are mutually exclusive; table order sets the
    /// file-dialog filter order, not claim precedence.
    pub is_rom: fn(&Path, &[u8]) -> bool,
    /// Title carried in the media's header, for families whose media has
    /// one; `None` falls back to the file stem.
    pub title_from_rom: fn(&[u8]) -> Option<String>,
    pub create_console: CreateConsole,
    /// gbtrace capture entry point for the `trace` subcommand; `None` for
    /// families without a trace backend.
    pub trace: Option<fn(TraceRequest)>,
}

/// The single classification point: the family whose media this is. Media
/// no family claims is unsupported.
pub fn family_for(path: &Path, rom: &[u8]) -> Option<&'static FamilyDescriptor> {
    FAMILIES.iter().find(|family| (family.is_rom)(path, rom))
}

/// Every registered family, in file-dialog filter order.
pub static FAMILIES: &[FamilyDescriptor] = &[
    FamilyDescriptor {
        platform: Platform::GameBoy,
        extensions: gb::ROM_EXTENSIONS,
        control_labels: &gb::CONTROL_LABELS,
        is_rom: gb::is_gb_rom,
        title_from_rom: gb::title_from_rom,
        create_console: gb::create_console,
        trace: Some(crate::trace::trace_gb),
    },
    FamilyDescriptor {
        platform: Platform::GameBoyColor,
        extensions: gb::GBC_ROM_EXTENSIONS,
        control_labels: &gb::CONTROL_LABELS,
        is_rom: gb::is_gbc_rom,
        title_from_rom: gb::title_from_rom,
        // The same factory serves both platforms: the header picks the core.
        create_console: gb::create_console,
        trace: Some(crate::trace::trace_gb),
    },
    FamilyDescriptor {
        platform: Platform::AtariVcs,
        extensions: vcs::ROM_EXTENSIONS,
        control_labels: &vcs::CONTROL_LABELS,
        is_rom: vcs::is_vcs_rom,
        title_from_rom: |_| None,
        create_console: |media| vcs::create_console(media.rom, media.fallback_title).ok(),
        trace: Some(crate::trace::trace_vcs),
    },
    #[cfg(feature = "sms")]
    FamilyDescriptor {
        platform: Platform::MasterSystem,
        extensions: sms::ROM_EXTENSIONS,
        control_labels: &sms::CONTROL_LABELS,
        is_rom: |path, _| sms::is_sms_rom(path),
        title_from_rom: |_| None,
        create_console: |media| sms::create_console(media.rom, media.fallback_title).ok(),
        trace: None,
    },
    #[cfg(feature = "nes")]
    FamilyDescriptor {
        platform: Platform::Nes,
        extensions: nes::ROM_EXTENSIONS,
        control_labels: &nes::CONTROL_LABELS,
        is_rom: |_, rom| nes::is_nes_rom(rom),
        title_from_rom: |_| None,
        create_console: |media| nes::create_console(media.rom, media.fallback_title).ok(),
        trace: Some(crate::trace::trace_nes),
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
    /// Latching console switches shown as in-play toggles (empty for
    /// families without any).
    fn console_switches(&self) -> &'static [ConsoleSwitch] {
        &[]
    }
    /// Stereo samples at 44.1 kHz — the seam's fixed rate. Families
    /// convert from their native rate on their own side.
    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)>;
    fn screen_display(&self) -> ScreenDisplay;
    fn capture_frame(&self, options: &CaptureOptions) -> FrameCapture;
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
    fn pane_family(&self) -> &'static panes::Family;
    /// Labels from the ROM's debug-symbol sidecar, if one was loaded.
    fn symbols(&self) -> Arc<SymbolTable> {
        empty_symbols()
    }
    /// Create a user label at an address; the system decides the bank from
    /// the current mapping.
    fn add_symbol(&mut self, _address: u16, _name: String) {}
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

    fn game_title(&self) -> String;
    fn battery_save(&self) -> Option<Vec<u8>> {
        None
    }
    fn frame_interval(&self) -> Duration;
    fn capture_frame(&self, options: &CaptureOptions) -> FrameCapture;
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
