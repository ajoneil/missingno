use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use iced::{
    Element, Length, Task,
    alignment::Vertical,
    widget::{Column, button, column, container, pane_grid, pick_list, row, text, text_input},
};

use missingno_session::SessionHandle;

use crate::app::system::Platform;
use crate::app::{
    self,
    console::ConsoleColors,
    ui::{
        fonts, icons, palette,
        sizes::{s, xs},
    },
};
use missingno_core::graphics::GraphicsView;
use missingno_core::inspect::{MemoryRegion, MemoryWindow, Section, Watch, WatchTerm};
use missingno_core::symbols::{Symbol, SymbolTable};
use missingno_core::system::RunningStatus;
use missingno_core::waveform::ChannelWave;
use missingno_gb::ppu::types::palette::PaletteChoice;
use missingno_iced::{Frame, ScreenView};

use disassembly::DisasmReadout;
use inspect::DebugView;
use panes::{DebuggerPanes, PaneContext};
use sidebar::Sidebar;

mod audio_scope;
mod disasm_rows;
mod disassembly;
mod graphics;
pub mod inspect;
mod layout;
pub(crate) mod memory;
pub mod panes;
mod screen;
pub(crate) mod sidebar;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BottomPanel {
    Breakpoints,
    Watchpoints,
    Labels,
}

/// The bus-access direction offered by the watchpoint add row's picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Read,
    Write,
}

impl AccessKind {
    const ALL: [AccessKind; 2] = [AccessKind::Read, AccessKind::Write];
}

impl std::fmt::Display for AccessKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AccessKind::Read => "Read",
            AccessKind::Write => "Write",
        })
    }
}

#[derive(Debug, Clone)]
pub enum BottomPaneMessage {
    Show(BottomPanel),
    Close(BottomPanel),
    Resize(pane_grid::ResizeEvent),
    Drag(pane_grid::DragEvent),
}

/// Vertical split ratio between main pane area and bottom panels.
const DEFAULT_SPLIT_RATIO: f32 = 0.75;

#[derive(Debug, Clone, Copy)]
enum MainSplit {
    Top,
    Bottom,
}

// Frame-carrying messages are produced once per frame; boxing buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Message {
    Step,
    StepOver,
    StepFrame,
    CaptureFrame,
    CaptureFrameTo(std::path::PathBuf),

    SetBreakpoint(u32),
    ClearBreakpoint(u32),
    BreakpointInputChanged(String),
    AddBreakpoint,

    RemoveWatchpoint(Watch),
    WatchpointInputChanged(String),
    WatchpointKindChanged(AccessKind),
    AddWatchpoint,
    /// Add a pre-built watch — the disassembly gutter's `{pc, bank}` compound.
    SetWatchpoint(Watch),

    RemoveLabel(Symbol),
    LabelAddressChanged(String),
    LabelNameChanged(String),
    AddLabel,

    /// Resolve a jump-to-address typed in the disassembly pane against the live
    /// core, which owns the region/bank mapping, then anchor the walk there.
    ResolveDisasmJump(String),

    BottomPane(BottomPaneMessage),
    MainSplitResize(pane_grid::ResizeEvent),

    Sidebar(sidebar::Message),
    Pane(panes::Message),
}

impl From<Message> for super::Message {
    fn from(val: Message) -> Self {
        super::Message::Debugger(val)
    }
}

/// The owned paused inspection readout, built in one [`SessionHandle::with_session`]
/// round-trip and cached so `view` (which borrows `&self`) can render the panes
/// from data that lives in the shell. Rebuilt at every core-mutation point;
/// absent while the session free-runs (the snapshot path renders instead).
struct PausedReadout {
    /// The Game Boy family's render palettes, `None` for other families.
    colors: Option<ConsoleColors>,
    /// One live-peek window per open memory pane, in selection order, each keyed
    /// by its base so a pane matches its own.
    memory_windows: Vec<MemoryWindow>,
    /// The disassembly rows, present only when a disassembly pane is open.
    disasm: Option<DisasmReadout>,
    /// The frozen per-channel waveform tail, `None` unless capture is on.
    waves: Option<Vec<ChannelWave>>,
    /// The decoded graphics surfaces, `None` unless graphics capture is on.
    graphics: Option<GraphicsView>,
    /// The structured machine-state sidebar sections.
    sidebar: Vec<Section>,
    /// The symbol table for the labels panel.
    symbols: Arc<SymbolTable>,
    /// The last watch the core stopped on, for the watchpoints panel.
    last_watch_hit: Option<Watch>,
}

pub struct Debugger {
    /// The client handle onto the session that owns this game's debugger core.
    handle: SessionHandle,
    /// Where the ROM's debug sidecars (.sym, .cdl) live; set on load.
    rom_path: Option<std::path::PathBuf>,
    /// The platform this debugger presents, captured at load; keys the pane
    /// registry and layout persistence.
    platform: Platform,
    /// UI copy of the breakpoint set, a display cache refreshed from the
    /// session. Held as bus addresses; a core masks to its own width.
    breakpoints: BTreeSet<u32>,
    /// UI copy of the watchpoint list, a display cache refreshed from the session.
    watchpoints: Vec<Watch>,
    /// Lightweight status published every frame while free-running; feeds the
    /// sidebar summary until the first full snapshot lands.
    last_status: Option<RunningStatus>,
    /// The per-vblank inspection snapshot the running panes render from.
    /// Boxed — a snapshot carries a full VRAM copy and shouldn't inflate the
    /// paused-path `Debugger` (and the `Game` enum) by that much.
    last_snapshot: Option<DebugView>,
    /// The memory viewer's interest window as of the last vblank, fed by the
    /// session while free-running; drives the running memory browser.
    last_memory_windows: Vec<MemoryWindow>,
    /// The core's region map, cached at build so the memory pane's running
    /// browser and jump-to-address work while the session free-runs. Owned
    /// because it is cart-dependent (a board with RAM adds a region).
    memory_regions: Vec<MemoryRegion>,
    /// The owned paused readout the panes render from while paused; `None` while
    /// free-running (the snapshot path renders instead).
    paused: Option<PausedReadout>,
    sidebar: Sidebar,
    panes: DebuggerPanes,
    frame: u64,
    bottom_panes: Option<pane_grid::State<BottomPanel>>,
    bottom_handles: HashMap<BottomPanel, pane_grid::Pane>,
    main_split: Option<pane_grid::State<MainSplit>>,
    breakpoint_input: String,
    watchpoint_input: String,
    watchpoint_kind: AccessKind,
    label_address_input: String,
    label_name_input: String,
}

/// The panes a platform presents. Every platform whose debugger this build can
/// construct registers a family behind the same feature gate.
fn pane_family(platform: Platform) -> &'static panes::Family {
    panes::family_for(platform).expect("a debugger's platform registers panes")
}

/// A parsed disassembly jump-to-address: a plain bus address, or a bank:window
/// pairing the live core maps to a synthetic bank-complete address.
#[derive(Debug, PartialEq, Eq)]
enum DisasmJump {
    Bus(u32),
    BankWindow { bank: u16, window: u32 },
}

/// Parse a jump-to-address field: `NN:AAAA` as a bank:window pairing, a plain
/// hex string (with optional `0x`/`$`) as a bus address. `None` when either
/// part fails to parse.
fn parse_disasm_jump(input: &str) -> Option<DisasmJump> {
    fn hex(text: &str) -> Option<u32> {
        let text = text.trim().trim_start_matches("0x").trim_start_matches('$');
        (!text.is_empty())
            .then(|| u32::from_str_radix(text, 16).ok())
            .flatten()
    }
    let input = input.trim();
    match input.split_once(':') {
        Some((bank, window)) => Some(DisasmJump::BankWindow {
            bank: u16::from_str_radix(bank.trim(), 16).ok()?,
            window: hex(window)?,
        }),
        None => hex(input).map(DisasmJump::Bus),
    }
}

impl Debugger {
    /// Build a shell over the session hosting a debugger core. `screen_view`
    /// carries the console's technology (fresh from a cold load, or transferred
    /// across a debugger toggle). Builds the initial paused readout so the first
    /// view has data.
    pub fn new(
        handle: SessionHandle,
        platform: Platform,
        memory_regions: Vec<MemoryRegion>,
        screen_view: ScreenView,
    ) -> Self {
        let panes = DebuggerPanes::with_screen(pane_family(platform), screen_view);
        let mut this = Self {
            handle,
            rom_path: None,
            platform,
            breakpoints: BTreeSet::new(),
            watchpoints: Vec::new(),
            last_status: None,
            last_snapshot: None,
            last_memory_windows: Vec::new(),
            memory_regions,
            paused: None,
            sidebar: Sidebar::new(),
            panes,
            frame: 0,
            bottom_panes: None,
            bottom_handles: HashMap::new(),
            main_split: None,
            breakpoint_input: String::new(),
            watchpoint_input: String::new(),
            watchpoint_kind: AccessKind::Write,
            label_address_input: String::new(),
            label_name_input: String::new(),
        };
        this.refresh_paused();
        this
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// Prepare to hand this game's console to a session of the other kind: save
    /// the debug sidecars while the session (and its handle) is still alive, then
    /// disarm the drop-time save so it cannot touch the handle once the session
    /// is consumed.
    pub fn prepare_handoff(&mut self) {
        self.save_sidecars();
        self.rom_path = None;
    }

    /// Make `refresh_paused` reachable to the app after a session-side mutation
    /// it drove directly (a state load, a run-loop stop).
    pub fn sync_paused(&mut self) {
        self.refresh_paused();
    }

    /// Load the ROM's debug sidecars, whatever the system keeps beside its media.
    pub fn load_sidecars(&mut self, rom_path: &std::path::Path) {
        let path = rom_path.to_path_buf();
        self.handle
            .with_session(move |s| s.debugger_mut().load_sidecars(&path));
        self.rom_path = Some(rom_path.to_path_buf());
        self.refresh_paused();
    }

    /// Rebuild the owned paused readout from the session in one round-trip, and
    /// push the current display frame to the screen pane. A no-op that clears the
    /// cache while free-running (the snapshot path renders instead).
    fn refresh_paused(&mut self) {
        if self.handle.is_running() {
            self.paused = None;
            return;
        }
        let palette = *self.panes.palette();
        let anchor = self.panes.disasm_anchor();
        let selections = self.panes.memory_selections();
        let regions = self.memory_regions.clone();
        let disasm_shown = self.panes.plane_shown(panes::DebuggerPane::Disassembly);
        let (readout, breakpoints, watches, frame, display) =
            self.handle.with_session(move |session| {
                let core = session.debugger();
                let colors = inspect::as_inspect_source(core.family_state())
                    .map(|source| source.colors(&palette));
                let memory_windows = selections
                    .iter()
                    .map(|&selection| memory::build_readout(core, &regions, selection))
                    .collect();
                let disasm = disasm_shown.then(|| disassembly::paused_readout(core, anchor));
                let readout = PausedReadout {
                    colors,
                    memory_windows,
                    disasm,
                    waves: core.channel_waves(),
                    graphics: core.graphics(),
                    sidebar: core.sidebar_sections(),
                    symbols: core.symbols(),
                    last_watch_hit: core.last_watch_hit(),
                };
                (
                    readout,
                    core.breakpoints(),
                    core.watches(),
                    session.frame(),
                    session.display_frame(),
                )
            });
        self.breakpoints = breakpoints;
        self.watchpoints = watches;
        self.frame = frame;
        self.paused = Some(readout);
        self.apply_frame(display);
    }

    fn save_sidecars(&self) {
        if let Some(rom_path) = &self.rom_path {
            let path = rom_path.clone();
            self.handle
                .with_session(move |s| s.debugger().save_sidecars(&path));
        }
    }

    /// The live screen state, taken to carry across a debugger→emulator toggle.
    pub fn take_screen_view(&self) -> ScreenView {
        self.panes.take_screen_view()
    }

    /// Update the screen pane from the session's latest-frame slot.
    pub fn apply_frame(&mut self, display: Frame) {
        self.panes
            .update(panes::Message::Broadcast(panes::PaneMessage::Screen(
                screen::Message::Update(std::sync::Arc::new(display)),
            )));
    }

    /// Update the live status shown while the session free-runs.
    pub fn apply_status(&mut self, status: RunningStatus) {
        self.frame = status.frame;
        self.last_status = Some(status);
    }

    /// Update the per-vblank inspection snapshot the running panes render from.
    pub fn apply_snapshot(&mut self, view: DebugView) {
        self.frame = view.frame();
        self.last_snapshot = Some(view);
    }

    /// Update the memory viewers' interest windows from the session's slot — one
    /// window per open memory pane.
    pub fn apply_memory_windows(&mut self, windows: Vec<MemoryWindow>) {
        self.last_memory_windows = windows;
    }

    /// The spans the open memory panes want peeked while free-running: the union
    /// of their views, resolved against the cached region map, as the session's
    /// engine command wants them. Empty when no memory pane is shown or the
    /// family has no region map.
    fn session_interests(&self) -> Vec<missingno_session::MemoryInterest> {
        memory::interests_for(&self.memory_regions, &self.panes.memory_selections())
            .into_iter()
            .map(|interest| missingno_session::MemoryInterest {
                start: interest.start,
                len: interest.len,
            })
            .collect()
    }

    /// Re-aim what the free-running core produces at the panes that are open:
    /// the vblank memory peek's spans, and whether waveform and graphics capture
    /// run at all. The session's debugger holds the capture state; the interest
    /// rides the engine command.
    fn aim_capture(&self) {
        let wants_waves = self.wants_wave_capture();
        let wants_graphics = self.wants_graphics_capture();
        self.handle.set_memory_interest(self.session_interests());
        self.handle.with_session(move |s| {
            s.set_wave_capture(wants_waves);
            s.set_graphics_capture(wants_graphics);
        });
    }

    /// Resolve a disassembly jump-to-address to a walk anchor. A `bank:addr` jump
    /// resolves through the live core's region/bank mapping to a synthetic
    /// bank-complete address; a plain hex address anchors directly in bus space.
    /// `None` for unparseable or unmapped input.
    fn resolve_disasm_jump(&self, input: &str) -> Option<u32> {
        match parse_disasm_jump(input)? {
            DisasmJump::Bus(address) => Some(address),
            DisasmJump::BankWindow { bank, window } => self
                .handle
                .with_session(move |s| s.debugger().locate_bank_window(bank, window)),
        }
    }

    /// Whether any consumer wants per-channel waveform capture on — currently
    /// the audio scope pane. Off frees the core's capture rings.
    fn wants_wave_capture(&self) -> bool {
        self.panes.plane_shown(panes::DebuggerPane::Audio)
    }

    /// Whether any consumer wants per-vblank graphics capture on — the tile,
    /// tile-map, and sprite panes. Off drops the snapshot's VRAM clone and its
    /// surface decode.
    fn wants_graphics_capture(&self) -> bool {
        self.panes.plane_shown(panes::DebuggerPane::Tiles)
            || self.panes.plane_shown(panes::DebuggerPane::TileMap)
            || self.panes.plane_shown(panes::DebuggerPane::Sprites)
    }

    /// The display technology the debugged console states.
    pub fn technology(&self) -> missingno_core::video::DisplayTechnology {
        self.panes.screen_technology()
    }

    fn set_breakpoint(&mut self, address: u32) {
        self.breakpoints.insert(address);
        self.handle.with_session(move |s| {
            let _ = s.set_breakpoint(address);
        });
    }

    fn clear_breakpoint(&mut self, address: u32) {
        self.breakpoints.remove(&address);
        self.handle
            .with_session(move |s| s.clear_breakpoint(address));
    }

    fn add_watchpoint(&mut self, watch: Watch) {
        if self.watchpoints.contains(&watch) {
            return;
        }
        self.watchpoints.push(watch.clone());
        self.handle
            .with_session(move |s| s.debugger_mut().add_watch(watch));
    }

    fn remove_watchpoint(&mut self, watch: &Watch) {
        self.watchpoints.retain(|w| w != watch);
        let watch = watch.clone();
        self.handle
            .with_session(move |s| s.debugger_mut().remove_watch(&watch));
    }

    fn display_after_step(&mut self) -> Task<app::Message> {
        self.refresh_paused();
        Task::none()
    }

    /// Run a paused-step command against the session, then drain and drop the
    /// audio it produced. The session's audio sink only drains in the run loop,
    /// so paused-step audio does not play — draining here keeps it from piling up
    /// and bursting when the game next resumes.
    fn step_and_drop(&self, step: impl FnOnce(&mut missingno_session::Session) + Send + 'static) {
        self.handle.with_session(move |session| {
            step(session);
            let _ = session.drain_audio_samples();
        });
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
            Message::Step => {
                self.step_and_drop(|s| {
                    s.step();
                });
                self.display_after_step()
            }
            Message::StepOver => {
                self.step_and_drop(|s| {
                    s.step_over();
                });
                self.display_after_step()
            }
            Message::StepFrame => {
                self.step_and_drop(|s| {
                    s.step_frame();
                });
                self.display_after_step()
            }
            Message::CaptureFrame => {
                let title = self.handle.with_session(|s| s.game_title());
                let title = title.to_lowercase().replace(' ', "_");
                let default_name = format!("{title}_frame{}.morepork", self.frame);

                let dialog = rfd::AsyncFileDialog::new()
                    .set_file_name(&default_name)
                    .add_filter("morepork", &["morepork"]);

                Task::perform(dialog.save_file(), |handle| match handle {
                    Some(h) => Message::CaptureFrameTo(h.path().to_path_buf()).into(),
                    None => app::Message::None,
                })
            }
            Message::CaptureFrameTo(path) => {
                let shown = path.display().to_string();
                let captured = self.handle.with_session(move |s| {
                    let captured = s.debugger_mut().capture_trace(&path).is_some();
                    // The capture steps a frame; drop its audio (paused, no sink).
                    let _ = s.drain_audio_samples();
                    captured
                });
                let notice = match captured {
                    true => {
                        self.refresh_paused();
                        format!("Trace captured to {shown}")
                    }
                    false => "Trace capture failed: this core has no capture backend".to_string(),
                };
                Task::done(app::Message::ShowNotice(notice))
            }

            Message::SetBreakpoint(address) => {
                self.set_breakpoint(address);
                Task::none()
            }
            Message::ClearBreakpoint(address) => {
                self.clear_breakpoint(address);
                Task::none()
            }
            Message::BreakpointInputChanged(input) => {
                self.breakpoint_input = input
                    .chars()
                    .filter(|c| c.is_ascii_hexdigit())
                    .take(4)
                    .collect();
                Task::none()
            }
            Message::AddBreakpoint => {
                if self.breakpoint_input.len() == 4 {
                    let address = u16::from_str_radix(&self.breakpoint_input, 16).unwrap();
                    self.set_breakpoint(address as u32);
                    self.breakpoint_input.clear();
                }
                Task::none()
            }

            Message::RemoveWatchpoint(watch) => {
                self.remove_watchpoint(&watch);
                Task::none()
            }
            Message::SetWatchpoint(watch) => {
                self.add_watchpoint(watch);
                Task::none()
            }
            Message::WatchpointInputChanged(input) => {
                self.watchpoint_input = input
                    .chars()
                    .filter(|c| c.is_ascii_hexdigit())
                    .take(4)
                    .collect();
                Task::none()
            }
            Message::WatchpointKindChanged(kind) => {
                self.watchpoint_kind = kind;
                Task::none()
            }
            Message::AddWatchpoint => {
                if self.watchpoint_input.len() == 4 {
                    let address = u16::from_str_radix(&self.watchpoint_input, 16).unwrap();
                    let key = match self.watchpoint_kind {
                        AccessKind::Read => "bus-read",
                        AccessKind::Write => "bus-write",
                    };
                    self.add_watchpoint(Watch::single(key, Some(address as u32), None));
                    self.watchpoint_input.clear();
                }
                Task::none()
            }

            Message::RemoveLabel(symbol) => {
                self.handle
                    .with_session(move |s| s.debugger_mut().remove_symbol(&symbol));
                self.save_sidecars();
                self.refresh_paused();
                Task::none()
            }
            Message::LabelAddressChanged(input) => {
                self.label_address_input = input
                    .chars()
                    .filter(|c| c.is_ascii_hexdigit())
                    .take(4)
                    .collect();
                Task::none()
            }
            Message::LabelNameChanged(input) => {
                self.label_name_input = input.chars().filter(|c| !c.is_whitespace()).collect();
                Task::none()
            }
            Message::AddLabel => {
                if self.label_address_input.len() == 4 && !self.label_name_input.is_empty() {
                    let address = u16::from_str_radix(&self.label_address_input, 16).unwrap();
                    let name = std::mem::take(&mut self.label_name_input);
                    self.handle
                        .with_session(move |s| s.debugger_mut().add_symbol(address as u32, name));
                    self.label_address_input.clear();
                    self.save_sidecars();
                    self.refresh_paused();
                }
                Task::none()
            }

            Message::BottomPane(msg) => {
                match msg {
                    BottomPaneMessage::Show(panel) => {
                        if !self.bottom_handles.contains_key(&panel) {
                            if let Some(panes) = &mut self.bottom_panes {
                                let (last, _) = panes.iter().last().unwrap();
                                let (handle, _) = panes
                                    .split(pane_grid::Axis::Vertical, *last, panel)
                                    .unwrap();
                                self.bottom_handles.insert(panel, handle);
                            } else {
                                let (panes, handle) = pane_grid::State::new(panel);
                                self.bottom_panes = Some(panes);
                                self.bottom_handles.insert(panel, handle);
                                self.create_main_split();
                            }
                        }
                    }
                    BottomPaneMessage::Close(panel) => {
                        if let Some(&handle) = self.bottom_handles.get(&panel) {
                            if self.bottom_handles.len() == 1 {
                                self.bottom_panes = None;
                                self.bottom_handles.clear();
                                self.main_split = None;
                            } else if let Some(panes) = &mut self.bottom_panes {
                                panes.close(handle);
                                self.bottom_handles.remove(&panel);
                            }
                        }
                    }
                    BottomPaneMessage::Resize(resize) => {
                        if let Some(panes) = &mut self.bottom_panes {
                            panes.resize(resize.split, resize.ratio);
                        }
                    }
                    BottomPaneMessage::Drag(drag) => {
                        if let pane_grid::DragEvent::Dropped { pane, target } = drag
                            && let Some(panes) = &mut self.bottom_panes
                        {
                            panes.drop(pane, target);
                        }
                    }
                }
                Task::none()
            }

            Message::MainSplitResize(resize) => {
                if let Some(split) = &mut self.main_split {
                    split.resize(resize.split, resize.ratio);
                }
                Task::none()
            }

            Message::Sidebar(message) => {
                self.sidebar.update(&message);
                Task::none()
            }

            Message::ResolveDisasmJump(input) => {
                if let Some(anchor) = self.resolve_disasm_jump(&input) {
                    self.panes
                        .update(panes::Message::Broadcast(panes::PaneMessage::Disassembly(
                            disassembly::Message::SetAnchor(Some(anchor)),
                        )));
                    self.refresh_paused();
                }
                Task::none()
            }

            Message::Pane(message) => {
                // Keep the memory pane's region cache fresh so its
                // jump-to-address resolves while the session free-runs; harmless
                // no-op for every other pane.
                self.panes
                    .update(panes::Message::Broadcast(panes::PaneMessage::Memory(
                        memory::Message::SetRegions(self.memory_regions.clone()),
                    )));
                self.panes.update(message);
                self.aim_capture();
                // Capture just changed what the core will decode; refresh so a
                // newly opened graphics pane fills without a step.
                self.refresh_paused();
                Task::none()
            }
        }
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.panes.set_palette(palette);
        // The DMG render palettes flow through the readout's colours; rebuild so
        // a palette change is reflected while paused.
        self.refresh_paused();
    }

    pub fn view(&self) -> Element<'_, app::Message> {
        let Some(paused) = self.paused.as_ref().filter(|_| !self.handle.is_running()) else {
            return self.running_view();
        };
        // One live readout per open memory pane, each matched back by its base.
        let memory_selections = self.panes.memory_selections();
        let ctx = PaneContext {
            colors: paused.colors.as_ref(),
            breakpoints: &self.breakpoints,
            watches: &self.watchpoints,
            memory: (!memory_selections.is_empty()).then(|| {
                memory::MemoryPaneData::paused(&self.memory_regions, &paused.memory_windows)
            }),
            disasm: paused.disasm.as_ref().map(disassembly::DisasmPaneData::new),
            waves: paused.waves.as_deref(),
            graphics: paused.graphics.as_ref(),
        };

        let center: Element<'_, app::Message> = if let Some(split_state) = &self.main_split {
            pane_grid(split_state, |_handle, zone, _maximized| {
                let content: Element<'_, app::Message> = match zone {
                    MainSplit::Top => self.panes.view(Some(ctx)),
                    MainSplit::Bottom => self.bottom_pane_grid(
                        self.bottom_panes
                            .as_ref()
                            .expect("bottom_panes must exist when main_split exists"),
                    ),
                };
                pane_grid::Content::new(content)
            })
            .on_resize(10.0, |resize| Message::MainSplitResize(resize).into())
            .spacing(s())
            .into()
        } else {
            self.panes.view(Some(ctx))
        };

        let sidebar = self
            .sidebar
            .view(paused.sidebar.clone(), paused.colors.as_ref());
        row![sidebar, center, self.icon_rail(),]
            .spacing(s())
            .padding(s())
            .into()
    }

    /// The view while the session free-runs. The screen pane stays live from the
    /// frame slot; every other pane and the sidebar render from the per-vblank
    /// inspection snapshot, falling back to titled placeholders and the
    /// [`RunningStatus`] summary until the first snapshot arrives.
    fn running_view(&self) -> Element<'_, app::Message> {
        let colors = self
            .last_snapshot
            .as_deref()
            .and_then(|snapshot| inspect::as_inspect_source(snapshot.family_state()))
            .map(|source| source.colors(self.panes.palette()));

        let center: Element<'_, app::Message> = if let Some(split_state) = &self.main_split {
            pane_grid(split_state, |_handle, zone, _maximized| {
                let content: Element<'_, app::Message> = match zone {
                    MainSplit::Top => self.running_center(colors.as_ref()),
                    MainSplit::Bottom => self.bottom_pane_grid(
                        self.bottom_panes
                            .as_ref()
                            .expect("bottom_panes must exist when main_split exists"),
                    ),
                };
                pane_grid::Content::new(content)
            })
            .on_resize(10.0, |resize| Message::MainSplitResize(resize).into())
            .spacing(s())
            .into()
        } else {
            self.running_center(colors.as_ref())
        };

        let sidebar = match self.last_snapshot.as_deref() {
            Some(snapshot) => self
                .sidebar
                .view(snapshot.sidebar_sections(), colors.as_ref()),
            // Before the first snapshot lands, summarise from the per-frame status.
            None => self.sidebar.running_summary(self.last_status.as_ref()),
        };

        row![sidebar, center, self.icon_rail(),]
            .spacing(s())
            .padding(s())
            .into()
    }

    /// The main pane area while running: snapshot-backed panes when a snapshot
    /// is present, titled placeholders otherwise.
    fn running_center<'a>(&'a self, colors: Option<&ConsoleColors>) -> Element<'a, app::Message> {
        let Some(snapshot) = self.last_snapshot.as_deref() else {
            return self.panes.view(None);
        };
        let disasm_readout = self
            .panes
            .plane_shown(panes::DebuggerPane::Disassembly)
            .then(|| disassembly::running_readout(snapshot))
            .flatten();
        // This vblank's captured windows; `None` unless capture is on.
        let waves = snapshot.channel_waves();
        // This vblank's decoded surfaces, borrowed from the snapshot; `None`
        // unless graphics capture is on.
        let graphics = snapshot.graphics();
        self.panes.view(Some(PaneContext {
            colors,
            breakpoints: &self.breakpoints,
            watches: &self.watchpoints,
            memory: self.running_memory(snapshot),
            disasm: disasm_readout
                .as_ref()
                .map(disassembly::DisasmPaneData::running),
            waves: waves.as_deref(),
            graphics,
        }))
    }

    /// The memory pane's running data: the region browser fed by the vblank
    /// interest window when the family has a region map, else the PC-anchored
    /// snapshot window as a fallback.
    fn running_memory<'a>(
        &'a self,
        snapshot: &'a dyn missingno_core::system::InspectSnapshot,
    ) -> Option<memory::MemoryPaneData<'a>> {
        if self.memory_regions.is_empty() {
            snapshot
                .memory_window()
                .map(memory::MemoryPaneData::running_window)
        } else {
            Some(memory::MemoryPaneData::running_browse(
                &self.memory_regions,
                &self.last_memory_windows,
            ))
        }
    }

    fn bottom_pane_grid<'a>(
        &'a self,
        state: &'a pane_grid::State<BottomPanel>,
    ) -> Element<'a, app::Message> {
        pane_grid(state, |_handle, panel, _maximized| {
            let content: Element<'_, app::Message> = match panel {
                BottomPanel::Breakpoints => self.breakpoints_content(),
                BottomPanel::Watchpoints => self.watchpoints_content(),
                BottomPanel::Labels => self.labels_content(),
            };

            panes::pane(panes::title_bar_plain(panel.label()), content)
        })
        .on_resize(10.0, |resize| {
            Message::BottomPane(BottomPaneMessage::Resize(resize)).into()
        })
        .on_drag(|drag| Message::BottomPane(BottomPaneMessage::Drag(drag)).into())
        .spacing(s())
        .into()
    }

    fn breakpoints_content(&self) -> Element<'_, app::Message> {
        let breakpoint_list = Column::from_iter(
            self.breakpoints
                .iter()
                .map(|&address| breakpoint_row(address)),
        );

        let input = text_input("Address (hex)...", &self.breakpoint_input)
            .font(fonts::monospace())
            .on_input(|value| Message::BreakpointInputChanged(value).into())
            .on_submit(Message::AddBreakpoint.into());

        column![breakpoint_list, input,]
            .spacing(s())
            .padding(s())
            .into()
    }

    fn watchpoints_content(&self) -> Element<'_, app::Message> {
        let watchpoint_list = Column::from_iter(self.watchpoints.iter().map(watchpoint_row));

        let address = text_input("Address (hex)...", &self.watchpoint_input)
            .font(fonts::monospace())
            .on_input(|value| Message::WatchpointInputChanged(value).into())
            .on_submit(Message::AddWatchpoint.into());

        let kind = pick_list(AccessKind::ALL, Some(self.watchpoint_kind), |kind| {
            Message::WatchpointKindChanged(kind).into()
        });

        let add_row = row![address, kind].spacing(s()).align_y(Vertical::Center);

        let hit = self
            .paused
            .as_ref()
            .and_then(|paused| paused.last_watch_hit.as_ref());
        let panel = match hit {
            Some(hit) => column![
                text(format!("hit: {}", watch_summary(hit))).font(fonts::monospace()),
                watchpoint_list,
                add_row,
            ],
            None => column![watchpoint_list, add_row],
        };

        panel.spacing(s()).padding(s()).into()
    }

    /// Label editing needs the paused readout's symbol table; while the session
    /// free-runs (no readout) the panel is read-only.
    fn labels_content(&self) -> Element<'_, app::Message> {
        let Some(paused) = &self.paused else {
            return column![text("Pause to edit labels").font(fonts::monospace()),]
                .spacing(s())
                .padding(s())
                .into();
        };
        let symbols = &paused.symbols;

        let user_rows = Column::from_iter(symbols.user_symbols().iter().map(label_row));

        let address = text_input("Address (hex)...", &self.label_address_input)
            .font(fonts::monospace())
            .on_input(|value| Message::LabelAddressChanged(value).into())
            .on_submit(Message::AddLabel.into());
        let name = text_input("Name...", &self.label_name_input)
            .font(fonts::monospace())
            .on_input(|value| Message::LabelNameChanged(value).into())
            .on_submit(Message::AddLabel.into());
        let add_row = row![address, name].spacing(s()).align_y(Vertical::Center);

        let total = text(format!(
            "{} labels · {} yours",
            symbols.len(),
            symbols.user_symbols().len()
        ))
        .font(fonts::monospace())
        .size(11.0)
        .color(palette::MUTED);

        column![total, user_rows, add_row]
            .spacing(s())
            .padding(s())
            .into()
    }

    fn icon_rail(&self) -> Element<'_, app::Message> {
        use icons::Icon;

        let pane_buttons = self.panes.available_panes().map(|pane| {
            let message = panes::Message::RailClick(pane).into();
            if pane.instanceable() {
                // Instanceable kinds show how many are open; each click opens one
                // more, so there is no on/off state to toggle.
                rail_icon_badged(
                    pane.icon(),
                    &pane.to_string(),
                    self.panes.instance_count(pane),
                    message,
                )
            } else {
                rail_icon(
                    pane.icon(),
                    &pane.to_string(),
                    self.panes.plane_shown(pane),
                    message,
                )
            }
        });

        let panel_buttons = [
            (BottomPanel::Breakpoints, Icon::Circle, "Breakpoints"),
            (BottomPanel::Watchpoints, Icon::Eye, "Watchpoints"),
            (BottomPanel::Labels, Icon::FileText, "Labels"),
        ]
        .into_iter()
        .map(|(panel, icon, label)| {
            let shown = self.bottom_handles.contains_key(&panel);
            let message = if shown {
                BottomPaneMessage::Close(panel)
            } else {
                BottomPaneMessage::Show(panel)
            };
            rail_icon(icon, label, shown, Message::BottomPane(message).into())
        });

        column![
            column(pane_buttons).spacing(xs()),
            iced::widget::Space::new().height(Length::Fill),
            column(panel_buttons).spacing(xs()),
        ]
        .padding([s(), xs()])
        .into()
    }

    fn create_main_split(&mut self) {
        let (mut state, top_handle) = pane_grid::State::new(MainSplit::Top);
        let (_, split) = state
            .split(pane_grid::Axis::Horizontal, top_handle, MainSplit::Bottom)
            .unwrap();
        state.resize(split, DEFAULT_SPLIT_RATIO);
        self.main_split = Some(state);
    }

    /// Whether the session is free-running — the session is the source of truth.
    pub fn running(&self) -> bool {
        self.handle.is_running()
    }

    pub fn run(&mut self) {
        self.aim_capture();
        self.handle.run();
        self.paused = None;
    }

    pub fn pause(&mut self) {
        self.handle.pause();
        self.refresh_paused();
    }

    pub fn reset(&mut self) {
        self.handle.reset();
        self.frame = 0;
        self.last_snapshot = None;
        self.last_status = None;
        self.last_memory_windows.clear();
        self.refresh_paused();
    }
}

/// Debug sidecars persist when the debugger goes away — same rationale as
/// the pane layout's drop-save.
impl Drop for Debugger {
    fn drop(&mut self) {
        self.save_sidecars();
    }
}

impl BottomPanel {
    fn label(&self) -> &'static str {
        match self {
            BottomPanel::Breakpoints => "Breakpoints",
            BottomPanel::Watchpoints => "Watchpoints",
            BottomPanel::Labels => "Labels",
        }
    }
}

fn rail_icon<'a>(
    icon: icons::Icon,
    label: &str,
    active: bool,
    message: app::Message,
) -> Element<'a, app::Message> {
    let color = if active {
        palette::PURPLE
    } else {
        palette::SURFACE2
    };
    let btn = button(icons::m_colored(icon, color))
        .on_press(message)
        .style(button::text);
    rail_tooltip(btn.into(), label)
}

/// An instanceable pane's rail button: the icon lit while any instance is open,
/// with a row of small accent dots beneath it — one per open instance.
fn rail_icon_badged<'a>(
    icon: icons::Icon,
    label: &str,
    count: usize,
    message: app::Message,
) -> Element<'a, app::Message> {
    let color = if count > 0 {
        palette::PURPLE
    } else {
        palette::SURFACE2
    };
    let icon_el: Element<'a, app::Message> = icons::m_colored(icon, color).into();
    let content: Element<'a, app::Message> = if count >= 1 {
        column![icon_el, instance_dots(count)]
            .spacing(2.0)
            .align_x(iced::alignment::Horizontal::Center)
            .into()
    } else {
        icon_el
    };
    let btn = button(content).on_press(message).style(button::text);
    rail_tooltip(btn.into(), label)
}

/// A row of small filled accent dots, one per open instance, capped at what the
/// rail button's width holds.
fn instance_dots<'a>(count: usize) -> Element<'a, app::Message> {
    const DOT: f32 = 3.0;
    const MAX_DOTS: usize = 4;
    let dots = (0..count.min(MAX_DOTS))
        .map(|_| container("").width(DOT).height(DOT).style(dot_style).into());
    row(dots).spacing(2.0).into()
}

fn dot_style(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(palette::PURPLE.into()),
        border: iced::Border::default().rounded(1.5),
        ..Default::default()
    }
}

/// The left-flyout tooltip shared by every rail button.
fn rail_tooltip<'a>(btn: Element<'a, app::Message>, label: &str) -> Element<'a, app::Message> {
    use crate::app::debugger::sidebar::help_tooltip;
    use iced::widget::tooltip;

    help_tooltip(btn, label.to_owned(), 13.0, tooltip::Position::Left)
}

fn label_row(symbol: &Symbol) -> Element<'static, app::Message> {
    container(
        row![
            button(icons::m(icons::Icon::Close))
                .on_press(Message::RemoveLabel(symbol.clone()).into())
                .style(button::text),
            text(format!(
                "{:02X}:{:04X} {}",
                symbol.bank, symbol.address, symbol.name
            ))
            .font(fonts::monospace())
        ]
        .align_y(Vertical::Center),
    )
    .into()
}

fn watchpoint_row(watch: &Watch) -> Element<'static, app::Message> {
    container(
        row![
            button(icons::m(icons::Icon::Close))
                .on_press(Message::RemoveWatchpoint(watch.clone()).into())
                .style(button::text),
            text(watch_summary(watch)).font(fonts::monospace())
        ]
        .align_y(Vertical::Center),
    )
    .into()
}

/// The address-watch summary the panel shows for a watch. The add row only
/// builds bus reads and writes, so those are the terms that reach here.
fn watch_summary(watch: &Watch) -> String {
    watch
        .terms
        .iter()
        .map(term_summary)
        .collect::<Vec<_>>()
        .join(" & ")
}

fn term_summary(term: &WatchTerm) -> String {
    let address = term.address.unwrap_or(0);
    match term.key.as_str() {
        "bus-read" => format!("read {address:#06X}"),
        "bus-write" => format!("write {address:#06X}"),
        "dma-read" => format!("dma read {address:#06X}"),
        "dma-write" => format!("dma write {address:#06X}"),
        other => match term.value {
            Some(value) => format!("{other} {value:#04X}"),
            None => format!("{other} {address:#06X}"),
        },
    }
}

fn breakpoint_row(address: u32) -> Element<'static, app::Message> {
    container(
        row![
            button(icons::breakpoint_enabled())
                .on_press(Message::ClearBreakpoint(address).into())
                .style(button::text),
            text(format!("{:04X}", address)).font(fonts::monospace())
        ]
        .align_y(Vertical::Center),
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::{DisasmJump, parse_disasm_jump};

    #[test]
    fn parses_bank_window_plain_hex_and_rejects_garbage() {
        // A bank:window pairing.
        assert_eq!(
            parse_disasm_jump("03:4123"),
            Some(DisasmJump::BankWindow {
                bank: 3,
                window: 0x4123
            })
        );
        // Plain hex, with and without sigils, is a bus address.
        assert_eq!(parse_disasm_jump("C000"), Some(DisasmJump::Bus(0xC000)));
        assert_eq!(parse_disasm_jump("$FF80"), Some(DisasmJump::Bus(0xFF80)));
        assert_eq!(parse_disasm_jump("0x0150"), Some(DisasmJump::Bus(0x0150)));
        // Unparseable input, on either side of the colon, rejects.
        assert_eq!(parse_disasm_jump("wram"), None);
        assert_eq!(parse_disasm_jump(""), None);
        assert_eq!(parse_disasm_jump("zz:4000"), None);
        assert_eq!(parse_disasm_jump("03:xy"), None);
    }
}
