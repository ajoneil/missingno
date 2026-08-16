//! The seam a console family implements. [`Machine`] is a flat list of hooks
//! over the family's core; [`MachineConsole`] and [`MachineDebugger`] carry the
//! control flow of [`SystemConsole`] and [`SystemDebugger`] exactly once, and
//! own the state that is generic across families — the displayed frame, the
//! breakpoint and watch stores, the save-state identity. The debugger wrapper
//! contains the console wrapper, so the plain-console surface exists in one
//! place. A family whose engine stops at sub-instruction points overrides the
//! run hooks instead of the traits.

use std::any::Any;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::HighPass;
use crate::cdl::CdlWindow;
use crate::graphics::GraphicsView;
use crate::inspect::{
    AddressDisplay, MemoryRegion, MemoryWindow, RegisterGroup, Section, Watch, Watchable,
};
use crate::isa::InstructionSet;
use crate::ports::{
    ControlDescriptor, PanelControl, PeripheralId, PlugError, PortDescriptor, PortId,
};
use crate::state::{StateRecord, SystemStateSchema};
use crate::state_file::{StateFrame, StateMeta, read_state_file, write_state_file};
use crate::symbols::{Symbol, SymbolTable};
use crate::system::{
    ControlId, ControlInput, DebugView, FrameOutcome, InspectSnapshot, RunningStatus, StateError,
    StepOutcome, SystemConsole, SystemDebugger,
};
use crate::video::{DisplayTechnology, Frame, RawFrame};
use crate::waveform::ChannelWave;

/// Bytes captured before the program counter — enough for the disassembly's
/// backward sweep — and the total span, its remainder covering the forward
/// window. Both fit inside the 16-bit address space these families wrap in.
const WINDOW_BEHIND: u16 = 128;
const WINDOW_LEN: u16 = 512;

/// The stops a run hook must honour: the PC breakpoints and the watch
/// conditions, held by the wrapper so every family stores them the same way.
#[derive(Default)]
pub struct StopSet {
    pub pc: BTreeSet<u32>,
    pub watches: Vec<Watch>,
}

/// Why a run hook returned.
pub enum CoreStop {
    /// Reached the boundary the hook was asked for: a completed frame, or the
    /// step-over's return address.
    Completed,
    Breakpoint,
    WatchHit(Watch),
    /// Ran out of instruction budget without reaching either.
    BudgetExhausted,
}

/// A run hook's result: why it stopped, and any frame the core completed on the
/// way — a step-over can finish a frame before it reaches its return address.
pub struct CoreRun<F> {
    pub stop: CoreStop,
    pub frame: Option<F>,
}

/// What a save state binds to, so a state can refuse a session it was not
/// written for. `None` at construction means the family's save states are
/// unavailable — the seam's default.
pub struct StateIdentity {
    /// SHA-256 of the loaded media, the digest the state file carries in hex.
    pub rom_fingerprint: [u8; 32],
}

/// A machine's state at an instruction boundary: the schema-keyed record, the
/// memory spans the schema names, and the displayed frame (informational — a
/// restored machine regenerates its display).
pub struct BoundaryState {
    pub record: StateRecord,
    pub memory: Vec<(&'static str, Vec<u8>)>,
    pub frame: Option<StateFrame>,
}

pub trait Machine: 'static {
    type Core: Send + 'static;
    type Frame;
    type InspectState: Clone + Send + 'static;

    /// Wall-clock duration of one emulated frame, for the pacing loop.
    const FRAME_INTERVAL: Duration;
    /// The pacing interval this console runs at, when it is not the family's
    /// nominal one — a set's broadcast standard decides it.
    fn frame_interval(_core: &Self::Core) -> Duration {
        Self::FRAME_INTERVAL
    }
    /// Instruction budget for one debugger-driven frame or step-over, so a
    /// core that never completes a frame cannot stall the UI.
    const RUN_BUDGET: u32;

    fn pc(core: &Self::Core) -> u16;
    /// Side-effect-free read of the CPU address space, for the memory viewer
    /// and the disassembly.
    fn peek(core: &Self::Core, address: u16) -> u8;
    /// Side-effect-free read of the debugger's address space, which a family may
    /// extend above the CPU bus with the bank-complete stores its
    /// [`memory_regions`](Machine::memory_regions) names.
    fn peek_region(core: &Self::Core, address: u32) -> u8 {
        Self::peek(core, address as u16)
    }
    /// The decode-for-display front end, when the family has one. `None`
    /// leaves the disassembly to fall back to raw bytes.
    fn instruction_set() -> Option<&'static dyn InstructionSet> {
        None
    }
    fn step_instruction(core: &mut Self::Core);
    /// The frame completed since the last take, if any.
    fn take_frame(core: &mut Self::Core) -> Option<Self::Frame>;
    /// Run up to one frame on the console's own budget.
    fn step_frame(core: &mut Self::Core) -> Option<Self::Frame>;
    fn power_cycle(core: &mut Self::Core);
    fn apply_control(core: &mut Self::Core, control: ControlId, input: ControlInput);
    /// The system's controller and expansion ports; empty for one that models
    /// none.
    fn ports() -> &'static [PortDescriptor] {
        &[]
    }
    /// The peripheral currently plugged into `port`.
    fn plugged(_core: &Self::Core, _port: PortId) -> Option<PeripheralId> {
        None
    }
    fn plug(
        _core: &mut Self::Core,
        _port: PortId,
        _peripheral: PeripheralId,
    ) -> Result<(), PlugError> {
        Err(PlugError::UnknownPort)
    }
    /// The system's built-in game controller, if it has one.
    fn integrated_controls() -> &'static [ControlDescriptor] {
        &[]
    }
    /// The controls on the system's shell: momentary buttons and switches.
    fn panel_controls() -> &'static [PanelControl] {
        &[]
    }
    fn drain_audio_samples(core: &mut Self::Core) -> Vec<(f32, f32)>;
    /// The coupling the board puts between the audio pads and the jack.
    fn audio_coupling() -> Option<HighPass> {
        None
    }

    /// The display device this system drives, pixel aspect included.
    fn video_out(core: &Self::Core) -> DisplayTechnology;
    /// A completed frame in the form the frontend displays it.
    fn display_frame(frame: &Self::Frame) -> Frame;
    /// What the display shows before the first frame completes.
    fn blank_display() -> Frame;
    /// The current frame in its pre-resolution domain — the values the accuracy
    /// references compare in. `None` falls back to the indices of the displayed
    /// frame.
    fn frame_raw(_core: &Self::Core) -> Option<RawFrame> {
        None
    }
    /// Whether this system renders through a user-selectable monochrome palette
    /// (DMG). Colour and TV systems leave it false.
    fn uses_monochrome_palette() -> bool {
        false
    }
    /// Whether the loaded media enables Super Game Boy enhancements.
    fn supports_sgb(_core: &Self::Core) -> bool {
        false
    }

    /// Whether the board's battery-backed save has changed since the last take.
    fn take_sram_dirty(_core: &mut Self::Core) -> bool {
        false
    }
    /// Serialized battery-backed save contents, if the media persists any.
    fn battery_save(_core: &Self::Core) -> Option<Vec<u8>> {
        None
    }

    /// The hardware-named state schema this system describes. `None` — the
    /// default — leaves it without save states.
    fn state_schema() -> Option<&'static SystemStateSchema> {
        None
    }
    /// The current machine state as a schema-keyed record, for the trace writer.
    fn read_state(_core: &Self::Core) -> Option<StateRecord> {
        None
    }
    /// The full boundary state a save file carries. A family refuses here — off
    /// an instruction boundary, or at a clock the record cannot name — rather
    /// than having the wrapper special-case it.
    fn capture_boundary(_core: &Self::Core) -> Result<BoundaryState, StateError> {
        Err(StateError::Unsupported)
    }
    /// Restore a machine from a parsed save file's record, memory spans, and
    /// framebuffer.
    fn restore_boundary(
        _core: &mut Self::Core,
        _record: &StateRecord,
        _memory: &[(String, Vec<u8>)],
        _frame: Option<&StateFrame>,
    ) -> Result<(), StateError> {
        Err(StateError::Unsupported)
    }

    /// The return address to run to when the instruction at PC is a call;
    /// `None` steps normally.
    fn step_over_target(core: &Self::Core) -> Option<u16>;
    /// The name of this core's sub-instruction step unit — a "dot" (T-cycle) on
    /// the Game Boy — or `None` when the finest step it exposes is a whole
    /// instruction.
    fn tick_name() -> Option<&'static str> {
        None
    }
    /// Advance one sub-instruction tick (see [`tick_name`](Machine::tick_name)).
    fn step_tick(_core: &mut Self::Core) {}
    /// Run until a frame completes or a stop fires. The default is an
    /// instruction loop over the PC breakpoints; a family whose engine
    /// evaluates stops at sub-instruction points overrides it and interprets
    /// the [`StopSet`] itself.
    fn run_frame(core: &mut Self::Core, stops: &StopSet) -> CoreRun<Self::Frame> {
        for _ in 0..Self::RUN_BUDGET {
            Self::step_instruction(core);
            if let Some(frame) = Self::take_frame(core) {
                return CoreRun {
                    stop: CoreStop::Completed,
                    frame: Some(frame),
                };
            }
            if stops.pc.contains(&(Self::pc(core) as u32)) {
                return CoreRun {
                    stop: CoreStop::Breakpoint,
                    frame: None,
                };
            }
        }
        CoreRun {
            stop: CoreStop::BudgetExhausted,
            frame: None,
        }
    }
    /// Run until the call at PC returns, or a stop fires.
    fn run_step_over(
        core: &mut Self::Core,
        stops: &StopSet,
        return_address: u16,
    ) -> CoreRun<Self::Frame> {
        let mut frame = None;
        for _ in 0..Self::RUN_BUDGET {
            Self::step_instruction(core);
            frame = Self::take_frame(core).or(frame);
            if Self::pc(core) == return_address {
                return CoreRun {
                    stop: CoreStop::Completed,
                    frame,
                };
            }
            if stops.pc.contains(&(Self::pc(core) as u32)) {
                return CoreRun {
                    stop: CoreStop::Breakpoint,
                    frame,
                };
            }
        }
        CoreRun {
            stop: CoreStop::BudgetExhausted,
            frame,
        }
    }

    /// The address map the debugger browses, named by role: the CPU bus, plus
    /// any bank-complete store the board exposes above it.
    fn memory_regions(_core: &Self::Core) -> Vec<MemoryRegion> {
        Vec::new()
    }
    /// The bank mapped at `address`, for a bank-prefixed disassembly row.
    fn bank_for(_core: &Self::Core, _address: u32) -> Option<u16> {
        None
    }
    /// How `address` presents in the disassembly's address column.
    fn present_address(core: &Self::Core, address: u32) -> AddressDisplay {
        AddressDisplay::bus(address, Self::bank_for(core, address))
    }
    /// The walk address whose disassembly row presents as `bank:window`.
    fn locate_bank_window(_core: &Self::Core, _bank: u16, _window: u32) -> Option<u32> {
        None
    }

    /// The watch conditions this system can name.
    fn watchables() -> &'static [Watchable] {
        &[]
    }
    /// Whether this system can evaluate `watch`. Defaults to every term naming
    /// one of its [`watchables`](Machine::watchables).
    fn watch_supported(watch: &Watch) -> bool {
        watch.terms.iter().all(|term| {
            Self::watchables()
                .iter()
                .any(|watchable| watchable.key == term.key)
        })
    }

    /// Labels from the ROM's debug-symbol sidecar, if one was loaded.
    fn symbols(_core: &Self::Core) -> Arc<SymbolTable> {
        crate::system::empty_symbols()
    }
    /// Create a user label at an address; the system decides the bank from the
    /// current mapping.
    fn add_symbol(_core: &mut Self::Core, _address: u32, _name: String) {}
    fn remove_symbol(_core: &mut Self::Core, _symbol: &Symbol) {}
    /// Code/data-log flags around the current instruction.
    fn cdl_window(_core: &Self::Core) -> CdlWindow {
        CdlWindow::default()
    }
    /// Load debug sidecars found beside the ROM.
    fn load_sidecars(_core: &mut Self::Core, _rom_path: &Path) {}
    /// Write updated debug sidecars back beside the ROM.
    fn save_sidecars(_core: &Self::Core, _rom_path: &Path) {}
    /// Step one frame while writing an execution trace to `path`.
    fn capture_trace(_core: &mut Self::Core, _path: &Path) -> Option<Frame> {
        None
    }

    /// Rebuild the typed inspection state from the core (peek-only).
    fn inspect(core: &Self::Core, frame_count: u64) -> Self::InspectState;
    /// Enable or disable the core's per-vblank graphics decode. The flag lives
    /// in the core, so the paused and running views read one producer.
    fn set_graphics_capture(_core: &mut Self::Core, _on: bool) {}
    /// The decoded graphics surfaces, or `None` when the family decodes none or
    /// capture is off.
    fn graphics_view(_core: &Self::Core) -> Option<GraphicsView> {
        None
    }
    /// Enable or disable the core's per-channel waveform capture.
    fn set_wave_capture(_core: &mut Self::Core, _on: bool) {}
    /// The captured per-channel waveforms, or `None` when the family captures
    /// none or capture is off.
    fn channel_waves(_core: &Self::Core) -> Option<Vec<ChannelWave>> {
        None
    }
    /// The register groups this system exposes for the schema-driven view.
    fn register_groups(_state: &Self::InspectState) -> Vec<RegisterGroup> {
        Vec::new()
    }
    /// The structured sidebar sections this system exposes, built from the
    /// typed state so the live and running views agree. Defaults to a single
    /// CPU section from the register groups; a system overrides to add its
    /// video section.
    fn sidebar_sections(state: &Self::InspectState) -> Vec<Section> {
        crate::inspect::default_sections(Self::register_groups(state))
    }
    /// An owned snapshot of the state, stamped with the UI's frame counter.
    fn snapshot(state: &Self::InspectState, frame: u64) -> DebugView;
    fn running_status(state: &Self::InspectState, frame: u64) -> RunningStatus;
}

pub struct MachineConsole<M: Machine> {
    core: M::Core,
    title: String,
    identity: Option<StateIdentity>,
    last_frame: Frame,
}

impl<M: Machine> MachineConsole<M> {
    pub fn new(core: M::Core, title: String) -> Self {
        MachineConsole {
            core,
            title,
            identity: None,
            last_frame: M::blank_display(),
        }
    }

    /// Bind this machine's save states to the loaded media. Without it the
    /// system has no save-state backend, whatever its schema says.
    pub fn with_identity(mut self, identity: StateIdentity) -> Self {
        self.identity = Some(identity);
        self
    }
}

impl<M: Machine> SystemConsole for MachineConsole<M> {
    fn step_frame(&mut self) -> FrameOutcome {
        let display = M::step_frame(&mut self.core).map(|frame| {
            self.last_frame = M::display_frame(&frame);
            self.last_frame.clone()
        });
        FrameOutcome {
            display,
            sram_dirty: M::take_sram_dirty(&mut self.core),
        }
    }

    fn reset(&mut self) {
        M::power_cycle(&mut self.core);
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        M::apply_control(&mut self.core, control, input);
    }

    fn ports(&self) -> &'static [PortDescriptor] {
        M::ports()
    }

    fn plugged(&self, port: PortId) -> Option<PeripheralId> {
        M::plugged(&self.core, port)
    }

    fn plug(&mut self, port: PortId, peripheral: PeripheralId) -> Result<(), PlugError> {
        M::plug(&mut self.core, port, peripheral)
    }

    fn integrated_controls(&self) -> &'static [ControlDescriptor] {
        M::integrated_controls()
    }

    fn panel_controls(&self) -> &'static [PanelControl] {
        M::panel_controls()
    }

    fn uses_monochrome_palette(&self) -> bool {
        M::uses_monochrome_palette()
    }

    fn supports_sgb(&self) -> bool {
        M::supports_sgb(&self.core)
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        M::drain_audio_samples(&mut self.core)
    }

    fn audio_coupling(&self) -> Option<HighPass> {
        M::audio_coupling()
    }

    fn screen_display(&self) -> Frame {
        self.last_frame.clone()
    }

    fn video_out(&self) -> DisplayTechnology {
        M::video_out(&self.core)
    }

    fn game_title(&self) -> String {
        self.title.clone()
    }

    fn battery_save(&self) -> Option<Vec<u8>> {
        M::battery_save(&self.core)
    }

    fn frame_interval(&self) -> Duration {
        M::frame_interval(&self.core)
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        let schema = M::state_schema()?;
        let fingerprint = hex_digest(&self.identity.as_ref()?.rom_fingerprint);
        let state = M::capture_boundary(&self.core).ok()?;
        let meta = StateMeta {
            system: schema.system,
            rom_sha256: Some(&fingerprint),
            emulator: "missingno",
            emulator_version: env!("CARGO_PKG_VERSION"),
        };
        write_state_file(&meta, &state.record, &state.memory, state.frame.as_ref()).ok()
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), StateError> {
        use crate::state_file::StateFileError;

        let schema = M::state_schema().ok_or(StateError::Unsupported)?;
        let identity = self.identity.as_ref().ok_or(StateError::Unsupported)?;
        let file = read_state_file(bytes).map_err(|error| match error {
            StateFileError::UnsupportedVersion(_) => StateError::VersionMismatch,
            _ => StateError::Corrupt,
        })?;
        if file.system != schema.system {
            return Err(StateError::WrongSystem);
        }
        if let Some(fingerprint) = &file.rom_sha256
            && *fingerprint != hex_digest(&identity.rom_fingerprint)
        {
            return Err(StateError::IncompatibleRom);
        }
        let record = schema
            .record_from(file.fields)
            .map_err(|_| StateError::Corrupt)?;
        M::restore_boundary(&mut self.core, &record, &file.memory, file.frame.as_ref())
    }

    fn state_schema(&self) -> Option<&'static SystemStateSchema> {
        M::state_schema()
    }

    fn read_state(&self) -> Option<StateRecord> {
        M::read_state(&self.core)
    }

    fn into_debugger(self: Box<Self>) -> Box<dyn SystemDebugger> {
        Box::new(MachineDebugger::new(*self))
    }
}

/// The hex spelling of a media digest, as the state file carries it.
fn hex_digest(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A machine under the seam's debugger: the console wrapper plus the stop
/// stores and the typed inspection state refreshed after every step.
pub struct MachineDebugger<M: Machine> {
    console: MachineConsole<M>,
    stops: StopSet,
    last_watch_hit: Option<Watch>,
    inspect: M::InspectState,
    frame_count: u64,
}

impl<M: Machine> MachineDebugger<M> {
    fn new(console: MachineConsole<M>) -> Self {
        let inspect = M::inspect(&console.core, 0);
        MachineDebugger {
            console,
            stops: StopSet::default(),
            last_watch_hit: None,
            inspect,
            frame_count: 0,
        }
    }

    fn refresh(&mut self) {
        self.inspect = M::inspect(&self.console.core, self.frame_count);
    }

    fn display(&mut self, frame: Option<M::Frame>) -> Option<Frame> {
        let frame = frame?;
        self.frame_count += 1;
        self.console.last_frame = M::display_frame(&frame);
        Some(self.console.last_frame.clone())
    }

    /// Land a run hook's result: cache its frame, refresh the inspection state,
    /// and say why the core stopped.
    fn land(&mut self, run: CoreRun<M::Frame>) -> StepOutcome {
        let display = self.display(run.frame);
        self.refresh();
        match run.stop {
            CoreStop::Completed => StepOutcome::Completed { frame: display },
            CoreStop::Breakpoint => StepOutcome::Breakpoint { frame: display },
            CoreStop::WatchHit(watch) => {
                self.last_watch_hit = Some(watch.clone());
                StepOutcome::WatchHit(watch)
            }
            CoreStop::BudgetExhausted => StepOutcome::BudgetExhausted,
        }
    }
}

impl<M: Machine> SystemConsole for MachineDebugger<M> {
    /// One frame under the debugger: the breakpoints still stop it, and the
    /// host learns why only through [`SystemDebugger::run_frame`].
    fn step_frame(&mut self) -> FrameOutcome {
        FrameOutcome {
            display: SystemDebugger::run_frame(self).into_frame(),
            sram_dirty: M::take_sram_dirty(&mut self.console.core),
        }
    }

    fn reset(&mut self) {
        self.console.reset();
        self.refresh();
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        self.console.set_control(control, input);
    }

    fn ports(&self) -> &'static [PortDescriptor] {
        self.console.ports()
    }

    fn plugged(&self, port: PortId) -> Option<PeripheralId> {
        self.console.plugged(port)
    }

    fn plug(&mut self, port: PortId, peripheral: PeripheralId) -> Result<(), PlugError> {
        self.console.plug(port, peripheral)
    }

    fn integrated_controls(&self) -> &'static [ControlDescriptor] {
        self.console.integrated_controls()
    }

    fn panel_controls(&self) -> &'static [PanelControl] {
        self.console.panel_controls()
    }

    fn uses_monochrome_palette(&self) -> bool {
        self.console.uses_monochrome_palette()
    }

    fn supports_sgb(&self) -> bool {
        self.console.supports_sgb()
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.console.drain_audio_samples()
    }

    fn audio_coupling(&self) -> Option<HighPass> {
        self.console.audio_coupling()
    }

    fn screen_display(&self) -> Frame {
        self.console.screen_display()
    }

    fn video_out(&self) -> DisplayTechnology {
        self.console.video_out()
    }

    fn game_title(&self) -> String {
        self.console.game_title()
    }

    fn battery_save(&self) -> Option<Vec<u8>> {
        self.console.battery_save()
    }

    fn frame_interval(&self) -> Duration {
        self.console.frame_interval()
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        self.console.save_state()
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), StateError> {
        let restored = self.console.load_state(bytes);
        self.refresh();
        restored
    }

    fn state_schema(&self) -> Option<&'static SystemStateSchema> {
        self.console.state_schema()
    }

    fn read_state(&self) -> Option<StateRecord> {
        self.console.read_state()
    }

    fn into_debugger(self: Box<Self>) -> Box<dyn SystemDebugger> {
        self
    }
}

impl<M: Machine> SystemDebugger for MachineDebugger<M> {
    fn step(&mut self) -> StepOutcome {
        M::step_instruction(&mut self.console.core);
        let frame = M::take_frame(&mut self.console.core);
        let display = self.display(frame);
        self.refresh();
        StepOutcome::Completed { frame: display }
    }

    fn step_over(&mut self) -> StepOutcome {
        let Some(return_address) = M::step_over_target(&self.console.core) else {
            return self.step();
        };
        let run = M::run_step_over(&mut self.console.core, &self.stops, return_address);
        self.land(run)
    }

    fn run_frame(&mut self) -> StepOutcome {
        let run = M::run_frame(&mut self.console.core, &self.stops);
        self.land(run)
    }

    fn tick_name(&self) -> Option<&'static str> {
        M::tick_name()
    }

    fn step_tick(&mut self) {
        M::step_tick(&mut self.console.core);
        self.refresh();
    }

    fn frame_raw(&self) -> Option<RawFrame> {
        M::frame_raw(&self.console.core).or_else(|| self.console.last_frame.to_raw())
    }

    fn set_wave_capture(&mut self, on: bool) {
        M::set_wave_capture(&mut self.console.core, on);
    }

    fn channel_waves(&self) -> Option<Vec<ChannelWave>> {
        M::channel_waves(&self.console.core)
    }

    fn set_graphics_capture(&mut self, on: bool) {
        M::set_graphics_capture(&mut self.console.core, on);
    }

    fn graphics(&self) -> Option<GraphicsView> {
        M::graphics_view(&self.console.core)
    }

    fn set_breakpoint(&mut self, address: u32) {
        self.stops.pc.insert(address);
    }

    fn clear_breakpoint(&mut self, address: u32) {
        self.stops.pc.remove(&address);
    }

    fn breakpoints(&self) -> BTreeSet<u32> {
        self.stops.pc.clone()
    }

    fn memory_regions(&self) -> Vec<MemoryRegion> {
        M::memory_regions(&self.console.core)
    }

    fn peek(&self, address: u32) -> u8 {
        M::peek_region(&self.console.core, address)
    }

    fn pc(&self) -> u32 {
        M::pc(&self.console.core) as u32
    }

    fn instruction_set(&self) -> Option<&dyn InstructionSet> {
        M::instruction_set()
    }

    fn bank_for(&self, address: u32) -> Option<u16> {
        M::bank_for(&self.console.core, address)
    }

    fn present_address(&self, address: u32) -> AddressDisplay {
        M::present_address(&self.console.core, address)
    }

    fn locate_bank_window(&self, bank: u16, window: u32) -> Option<u32> {
        M::locate_bank_window(&self.console.core, bank, window)
    }

    fn watchables(&self) -> &'static [Watchable] {
        M::watchables()
    }

    fn add_watch(&mut self, watch: Watch) {
        if M::watch_supported(&watch) && !self.stops.watches.contains(&watch) {
            self.stops.watches.push(watch);
        }
    }

    fn remove_watch(&mut self, watch: &Watch) {
        self.stops.watches.retain(|held| held != watch);
    }

    fn watches(&self) -> Vec<Watch> {
        self.stops.watches.clone()
    }

    fn last_watch_hit(&self) -> Option<Watch> {
        self.last_watch_hit.clone()
    }

    fn symbols(&self) -> Arc<SymbolTable> {
        M::symbols(&self.console.core)
    }

    fn add_symbol(&mut self, address: u32, name: String) {
        M::add_symbol(&mut self.console.core, address, name);
    }

    fn remove_symbol(&mut self, symbol: &Symbol) {
        M::remove_symbol(&mut self.console.core, symbol);
    }

    fn cdl_window(&self) -> CdlWindow {
        M::cdl_window(&self.console.core)
    }

    fn load_sidecars(&mut self, rom_path: &Path) {
        M::load_sidecars(&mut self.console.core, rom_path);
    }

    fn save_sidecars(&self, rom_path: &Path) {
        M::save_sidecars(&self.console.core, rom_path);
    }

    fn family_state(&self) -> &dyn Any {
        &self.inspect
    }

    fn register_groups(&self) -> Vec<RegisterGroup> {
        M::register_groups(&self.inspect)
    }

    fn sidebar_sections(&self) -> Vec<Section> {
        M::sidebar_sections(&self.inspect)
    }

    fn snapshot(&self, frame: u64) -> DebugView {
        let pc = M::pc(&self.console.core);
        let base = pc.wrapping_sub(WINDOW_BEHIND);
        let bytes = (0..WINDOW_LEN)
            .map(|i| M::peek(&self.console.core, base.wrapping_add(i)))
            .collect();
        Box::new(MachineSnapshot {
            inner: M::snapshot(&self.inspect, frame),
            pc,
            memory: MemoryWindow {
                base: base as u32,
                bytes,
            },
            instruction_set: M::instruction_set(),
            graphics: M::graphics_view(&self.console.core),
            waves: M::channel_waves(&self.console.core),
        })
    }

    fn running_status(&self, frame: u64) -> RunningStatus {
        M::running_status(&self.inspect, frame)
    }

    fn capture_trace(&mut self, path: &Path) -> Option<Frame> {
        M::capture_trace(&mut self.console.core, path)
    }

    fn into_console(self: Box<Self>) -> Box<dyn SystemConsole> {
        Box::new(self.console)
    }
}

/// Wraps a family's per-frame snapshot with the shared running-view fuel the
/// seam captures generically: the program counter, a PC-anchored memory window,
/// the instruction set, and whatever the core's capture hooks yielded. The
/// family's own state stays reachable through `family_state` for its typed
/// panes.
struct MachineSnapshot {
    inner: DebugView,
    pc: u16,
    memory: MemoryWindow,
    instruction_set: Option<&'static dyn InstructionSet>,
    graphics: Option<GraphicsView>,
    waves: Option<Vec<ChannelWave>>,
}

impl InspectSnapshot for MachineSnapshot {
    fn frame(&self) -> u64 {
        self.inner.frame()
    }
    fn family_state(&self) -> &dyn Any {
        self.inner.family_state()
    }
    fn register_groups(&self) -> Vec<RegisterGroup> {
        self.inner.register_groups()
    }
    fn sidebar_sections(&self) -> Vec<Section> {
        self.inner.sidebar_sections()
    }
    fn memory_window(&self) -> Option<&MemoryWindow> {
        Some(&self.memory)
    }
    fn pc(&self) -> Option<u32> {
        Some(self.pc as u32)
    }
    fn symbols(&self) -> Option<&SymbolTable> {
        self.inner.symbols()
    }
    fn cdl_window(&self) -> Option<&CdlWindow> {
        self.inner.cdl_window()
    }
    fn bank_for(&self, address: u32) -> Option<u16> {
        self.inner.bank_for(address)
    }
    fn present_address(&self, address: u32) -> AddressDisplay {
        self.inner.present_address(address)
    }
    fn instruction_set(&self) -> Option<&dyn InstructionSet> {
        self.instruction_set
    }
    fn channel_waves(&self) -> Option<Vec<ChannelWave>> {
        self.waves.clone()
    }
    fn graphics(&self) -> Option<&GraphicsView> {
        self.graphics.as_ref()
    }
}
