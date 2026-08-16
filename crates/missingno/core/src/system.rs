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
use crate::graphics::GraphicsView;
use crate::inspect::{MemoryRegion, MemoryWindow, RegisterGroup, Section, Watch, Watchable};
use crate::isa::InstructionSet;
use crate::ports::{
    ControlDescriptor, PanelControl, PeripheralId, PlugError, PortDescriptor, PortId,
};
use crate::state::{StateRecord, SystemStateSchema};
use crate::symbols::{Symbol, SymbolTable};
use crate::video::{DisplayTechnology, Frame, RawFrame};
use crate::waveform::ChannelWave;

/// Where a control physically lives on the machine.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum ControlSite {
    /// The console's own game controller — the Game Boy's pad.
    Integrated,
    /// The console shell: panel buttons and latching switches.
    Panel,
    /// The peripheral plugged into a port.
    Port(PortId),
}

/// What a control does. Roles rather than numbers, so a binding means the same
/// thing on every console and follows a peripheral into any port.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum ControlRole {
    Start,
    Select,
    /// The console's play/pause button — the Master System's Pause.
    Pause,
    /// The console's game-reset button — the VCS's Reset.
    Reset,
    /// Action button n: 0 is A / Fire / Button 1, 1 is B / Button 2. A paddle
    /// pair numbers its buttons the same way.
    Action(u8),
    Up,
    Down,
    Left,
    Right,
    /// Analog knob n on the site's peripheral.
    Knob(u8),
    /// Latching panel switch n.
    Toggle(u8),
    /// Key n of a matrix keypad, row-major from its top-left: the VCS
    /// keyboard controller's 0-8 are 1-9, then 9 = `*`, 10 = 0, 11 = `#`.
    Key(u8),
}

/// A control on a running machine: what it does, and where it sits. The
/// family's descriptors — [`integrated_controls`](SystemConsole::integrated_controls),
/// [`panel_controls`](SystemConsole::panel_controls), and the controls of the
/// peripheral plugged into each [`port`](SystemConsole::ports) — state which
/// ones exist.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub struct ControlId {
    pub site: ControlSite,
    pub role: ControlRole,
}

impl ControlSite {
    /// The site's spelling on a text surface: `integrated`, `panel`, `port0`.
    pub fn name(&self) -> String {
        match self {
            ControlSite::Integrated => "integrated".to_string(),
            ControlSite::Panel => "panel".to_string(),
            ControlSite::Port(port) => format!("port{}", port.0),
        }
    }

    pub fn parse(name: &str) -> Option<ControlSite> {
        match name {
            "integrated" => Some(ControlSite::Integrated),
            "panel" => Some(ControlSite::Panel),
            _ => name
                .strip_prefix("port")?
                .parse()
                .ok()
                .map(|port| ControlSite::Port(PortId(port))),
        }
    }
}

impl ControlRole {
    /// The role's spelling on a text surface: `start`, `action0`, `toggle2`.
    pub fn name(&self) -> String {
        match self {
            ControlRole::Start => "start".to_string(),
            ControlRole::Select => "select".to_string(),
            ControlRole::Pause => "pause".to_string(),
            ControlRole::Reset => "reset".to_string(),
            ControlRole::Action(n) => format!("action{n}"),
            ControlRole::Up => "up".to_string(),
            ControlRole::Down => "down".to_string(),
            ControlRole::Left => "left".to_string(),
            ControlRole::Right => "right".to_string(),
            ControlRole::Knob(n) => format!("knob{n}"),
            ControlRole::Toggle(n) => format!("toggle{n}"),
            ControlRole::Key(n) => format!("key{n}"),
        }
    }

    pub fn parse(name: &str) -> Option<ControlRole> {
        let numbered = |prefix: &str| name.strip_prefix(prefix).and_then(|n| n.parse().ok());
        match name {
            "start" => Some(ControlRole::Start),
            "select" => Some(ControlRole::Select),
            "pause" => Some(ControlRole::Pause),
            "reset" => Some(ControlRole::Reset),
            "up" => Some(ControlRole::Up),
            "down" => Some(ControlRole::Down),
            "left" => Some(ControlRole::Left),
            "right" => Some(ControlRole::Right),
            _ => numbered("action")
                .map(ControlRole::Action)
                .or_else(|| numbered("knob").map(ControlRole::Knob))
                .or_else(|| numbered("toggle").map(ControlRole::Toggle))
                .or_else(|| numbered("key").map(ControlRole::Key)),
        }
    }
}

impl std::fmt::Display for ControlId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.site.name(), self.role.name())
    }
}

impl ControlId {
    pub const fn integrated(role: ControlRole) -> ControlId {
        ControlId {
            site: ControlSite::Integrated,
            role,
        }
    }

    pub const fn panel(role: ControlRole) -> ControlId {
        ControlId {
            site: ControlSite::Panel,
            role,
        }
    }

    pub const fn port(port: PortId, role: ControlRole) -> ControlId {
        ControlId {
            site: ControlSite::Port(port),
            role,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlInput {
    Digital(bool),
    /// Normalised 0.0-1.0 (paddle knobs, pots).
    Axis(f32),
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
    /// The state was written for a different console family (a DMG state into a
    /// CGB session, or vice versa).
    WrongSystem,
    /// The state was written for a different ROM.
    IncompatibleRom,
    /// The container version is not the one this build implements.
    VersionMismatch,
    /// The state data is malformed.
    Corrupt,
    /// A save or restore was attempted off an instruction boundary — the CPU is
    /// mid-instruction, carrying micro-sequencer residue a boundary record does
    /// not name.
    NotAtBoundary,
    /// The state was taken with the CGB double-speed clock engaged. A boundary
    /// restore cannot reconstruct the free-running dot-phase alignment that a
    /// real speed switch left, so restore is limited to single-speed boundaries.
    DoubleSpeedBoundary,
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            StateError::Unsupported => "save states are not supported for this system",
            StateError::WrongSystem => "save state was written for a different system",
            StateError::IncompatibleRom => "save state was written for a different ROM",
            StateError::VersionMismatch => "save state uses an unsupported format version",
            StateError::Corrupt => "save state data is corrupt",
            StateError::NotAtBoundary => {
                "save state cannot be taken or restored mid-instruction; step to an instruction boundary"
            }
            StateError::DoubleSpeedBoundary => {
                "save state was taken at double speed; restore supports single-speed boundaries only"
            }
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
    /// The register groups as captured, for schema-driven panes while the core
    /// runs on the emulation thread.
    fn register_groups(&self) -> Vec<RegisterGroup> {
        Vec::new()
    }
    /// The sidebar sections as captured, so the schema-driven sidebar renders
    /// the same content while the core runs as it does paused. Defaults to a
    /// single CPU section from the captured register groups.
    fn sidebar_sections(&self) -> Vec<Section> {
        crate::inspect::default_sections(self.register_groups())
    }
    /// A span of memory captured this vblank, for the memory viewer while the
    /// core runs. `None` when the family captures no such window — the viewer
    /// then shows only that full memory needs a pause.
    fn memory_window(&self) -> Option<&MemoryWindow> {
        None
    }
    /// Where the program counter stood when this snapshot was taken, so the
    /// running disassembly can anchor on it. `None` when the family keeps no
    /// program counter.
    fn pc(&self) -> Option<u32> {
        None
    }
    /// The debug-symbol table as captured, for the running disassembly's label
    /// rows. `None` when the family loads no symbols.
    fn symbols(&self) -> Option<&SymbolTable> {
        None
    }
    /// The code/data-log flags around the captured program counter, for the
    /// running disassembly's data-byte rows. `None` when the family logs none.
    fn cdl_window(&self) -> Option<&CdlWindow> {
        None
    }
    /// The bank mapped at `address`, for a bank-prefixed running-disassembly
    /// row. `None` outside any switchable region, or for a family without one.
    fn bank_for(&self, _address: u32) -> Option<u16> {
        None
    }
    /// How `address` presents in the disassembly's address column. Defaults to a
    /// plain bus row carrying [`bank_for`](Self::bank_for); a family with a
    /// synthetic bank-complete space overrides to present its rows as bank:window.
    fn present_address(&self, address: u32) -> crate::inspect::AddressDisplay {
        crate::inspect::AddressDisplay::bus(address, self.bank_for(address))
    }
    /// The decode-for-display front end for the running disassembly. `None`
    /// when the family has no instruction set (its disassembly falls back to
    /// raw bytes).
    fn instruction_set(&self) -> Option<&dyn InstructionSet> {
        None
    }
    /// The per-channel waveform windows captured this vblank, for the audio
    /// scope while the core runs. `None` when the family captures no waveforms,
    /// or capture is disabled.
    fn channel_waves(&self) -> Option<Vec<ChannelWave>> {
        None
    }
    /// The decoded graphics surfaces (tile atlases, maps, object table) captured
    /// this vblank, for the graphics panes while the core runs. Borrowed from the
    /// snapshot so the per-frame render does not clone it. `None` when the family
    /// has no such surfaces, or graphics capture is disabled.
    fn graphics(&self) -> Option<&GraphicsView> {
        None
    }
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
    /// The console's controller and expansion ports; empty for a family that
    /// models none.
    fn ports(&self) -> &'static [PortDescriptor] {
        &[]
    }
    /// The peripheral currently plugged into `port`.
    fn plugged(&self, _port: PortId) -> Option<PeripheralId> {
        None
    }
    /// Plug a console-constructible peripheral into a port. A
    /// [`Provider::Host`](crate::ports::Provider::Host) kind arrives as an
    /// object from the host instead, and is refused here.
    fn plug(&mut self, _port: PortId, _peripheral: PeripheralId) -> Result<(), PlugError> {
        Err(PlugError::UnknownPort)
    }
    /// The console's built-in game controller — the Game Boy's pad. Empty for
    /// a console whose controllers all arrive through ports.
    fn integrated_controls(&self) -> &'static [ControlDescriptor] {
        &[]
    }
    /// The controls on the console shell: momentary buttons and the latching
    /// switches the UI renders as in-play toggles.
    fn panel_controls(&self) -> &'static [PanelControl] {
        &[]
    }
    /// Whether this console renders through a user-selectable monochrome
    /// palette (DMG). The play-mode Display panel shows its palette picker
    /// only when true; colour and TV systems return false.
    fn uses_monochrome_palette(&self) -> bool {
        false
    }
    /// Whether the loaded media enables Super Game Boy enhancements (the SGB
    /// palette override). Only meaningful for the Game Boy; others return false.
    fn supports_sgb(&self) -> bool {
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
    /// The display device this console drives: a fixed-size LCD, or a CRT.
    fn video_out(&self) -> DisplayTechnology;
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
    /// The hardware-named state schema this console describes, if it authors
    /// one. The save-state bridge and the trace writer key their records on its
    /// fields; a console without a schema returns `None`.
    fn state_schema(&self) -> Option<&'static SystemStateSchema> {
        None
    }
    /// Read the current machine state into a record keyed by the schema's field
    /// names — the bridge's capture side. `None` for a console without a schema
    /// or before its bridge is wired.
    fn read_state(&self) -> Option<StateRecord> {
        None
    }
    /// Convert to the debugger-backed form.
    fn into_debugger(self: Box<Self>) -> Box<dyn SystemDebugger>;
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

/// A console under a debugger: stepping, breakpoints, and inspection. Every
/// debugger is also a [`SystemConsole`], so a host drives it exactly as it
/// drives a bare console and reaches for this surface only to inspect.
///
/// Watchpoints, symbols, code/data logging, and trace capture default to
/// absent — a family implements only the backends it has. Breakpoints and
/// peeks cross the seam as `u32`; a core masks them to its own bus width.
pub trait SystemDebugger: SystemConsole {
    fn step(&mut self) -> StepOutcome;
    fn step_over(&mut self) -> StepOutcome;
    /// Run until the next frame or a stop — the debugger's counterpart to
    /// [`SystemConsole::step_frame`], reporting why it stopped.
    fn run_frame(&mut self) -> StepOutcome;
    /// The name of this core's sub-instruction step unit — a "dot" (T-cycle) on
    /// the Game Boy, a "colour clock" on the VCS — or `None` when the finest
    /// step the core exposes is a whole instruction. A transport offers
    /// sub-instruction stepping only when this is `Some`.
    fn tick_name(&self) -> Option<&'static str> {
        None
    }
    /// Advance one sub-instruction tick (see [`tick_name`](Self::tick_name)).
    /// A core without sub-instruction granularity does nothing.
    fn step_tick(&mut self) {}
    /// The current frame in its pre-resolution domain (the values the accuracy
    /// references compare in), or `None` when the family has no such surface.
    /// Defaults to the palette indices of an indexed frame; a family whose
    /// screen resolves before the seam overrides with its own raw domain.
    fn frame_raw(&self) -> Option<RawFrame> {
        self.screen_display().to_raw()
    }
    /// Enable or disable per-channel waveform capture. Interest-gated: capture
    /// stays off — and costs nothing — until a consumer turns it on. A family
    /// without a capture backend does nothing.
    fn set_wave_capture(&mut self, _on: bool) {}
    /// The current per-channel waveform windows, for the audio scope while
    /// paused. `None` when the family captures no waveforms, or capture is
    /// disabled.
    fn channel_waves(&self) -> Option<Vec<ChannelWave>> {
        None
    }
    /// Enable or disable graphics-surface capture. Interest-gated like
    /// [`set_wave_capture`](Self::set_wave_capture): capture stays off — and
    /// the per-vblank tile/map/object decode costs nothing — until a consumer
    /// turns it on. A family without a graphics backend does nothing.
    fn set_graphics_capture(&mut self, _on: bool) {}
    /// The current decoded graphics surfaces, for the graphics panes while
    /// paused. `None` when the family has none, or graphics capture is disabled.
    fn graphics(&self) -> Option<GraphicsView> {
        None
    }

    fn set_breakpoint(&mut self, address: u32);
    fn clear_breakpoint(&mut self, address: u32);
    fn breakpoints(&self) -> BTreeSet<u32>;

    /// The register groups this core exposes for the registers view.
    fn register_groups(&self) -> Vec<RegisterGroup> {
        Vec::new()
    }
    /// The structured left-column sidebar this core exposes. Defaults to a
    /// single CPU section from [`register_groups`](Self::register_groups); a
    /// family overrides to add its video and system sections.
    fn sidebar_sections(&self) -> Vec<Section> {
        crate::inspect::default_sections(self.register_groups())
    }
    /// The CPU-visible address map, named by role. Owned because the list is
    /// cart-dependent — a board with RAM contributes a region the bare console
    /// does not — even though each region's `name` stays a static string.
    fn memory_regions(&self) -> Vec<MemoryRegion> {
        Vec::new()
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
    /// The bank currently mapped at `address`, for a bank-prefixed disassembly
    /// row. `None` outside any switchable region, or for a core without one.
    fn bank_for(&self, _address: u32) -> Option<u16> {
        None
    }
    /// How `address` presents in the disassembly's address column. Defaults to a
    /// plain bus row carrying [`bank_for`](Self::bank_for); a core with a
    /// synthetic bank-complete space overrides to present its rows as bank:window.
    fn present_address(&self, address: u32) -> crate::inspect::AddressDisplay {
        crate::inspect::AddressDisplay::bus(address, self.bank_for(address))
    }
    /// The walk address whose disassembly row presents as `bank:window` — the
    /// synthetic bank-complete address for a banked region, for jump-to-address.
    /// `None` when no region presents that pairing. Inverse of
    /// [`present_address`](Self::present_address) over the synthetic space.
    fn locate_bank_window(&self, _bank: u16, _window: u32) -> Option<u32> {
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

    /// Step one frame while writing an execution trace to `path`; `None` when
    /// the system has no capture backend or capture fails.
    fn capture_trace(&mut self, _path: &Path) -> Option<Frame> {
        None
    }

    /// Hand back a plain console, dropping the debugger's per-step bookkeeping.
    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole>;
}

/// The shared empty table behind the default [`SystemDebugger::symbols`].
pub(crate) fn empty_symbols() -> Arc<SymbolTable> {
    use std::sync::OnceLock;
    static EMPTY: OnceLock<Arc<SymbolTable>> = OnceLock::new();
    EMPTY
        .get_or_init(|| Arc::new(SymbolTable::default()))
        .clone()
}
