use std::collections::{BTreeSet, HashMap};

use iced::{
    Element, Length, Task,
    alignment::Vertical,
    widget::{Column, button, column, container, pane_grid, pick_list, row, text, text_input},
};

use crate::app::system::{ControlId, ControlInput, Platform, StepOutcome};
use crate::app::{
    self,
    console::ConsoleColors,
    emu_thread::{DebuggerPayload, EmuCommand, EmuHandle, RunningStatus},
    emulator::Emulator,
    library::activity::{CaptureOptions, FrameCapture},
    screen::{Frame, ScreenView},
    system::{SystemConsole, SystemDebugger},
    ui::{
        fonts, icons, palette,
        sizes::{s, xs},
    },
};
use missingno_core::inspect::{MemoryRegion, MemoryWindow, Watch, WatchTerm};
use missingno_core::symbols::Symbol;
use missingno_gb::ppu::types::palette::PaletteChoice;
use missingno_gb::ppu::types::tiles::TileMapId;

use inspect::{DebugView, GbPaneContext};
use panes::{DebuggerPanes, PaneContext};
use sidebar::Sidebar;

mod audio_scope;
mod disasm_rows;
mod disassembly;
mod graphics;
pub mod inspect;
mod layout;
pub(crate) mod memory;
#[cfg(feature = "nes")]
pub(crate) mod nes;
pub mod panes;
mod screen;
pub(crate) mod sidebar;
#[cfg(feature = "sms")]
pub(crate) mod sms;

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

    RemoveLabel(Symbol),
    LabelAddressChanged(String),
    LabelNameChanged(String),
    AddLabel,

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

pub struct Debugger {
    /// The core (console + breakpoints) — `None` while it runs on the emu thread.
    debugger: Option<Box<dyn SystemDebugger>>,
    /// Where the ROM's debug sidecars (.sym, .cdl) live; set on load.
    rom_path: Option<std::path::PathBuf>,
    /// The platform this debugger presents, captured at load; keys the pane
    /// registry and layout persistence.
    platform: Platform,
    /// UI copy of the breakpoint set, kept editable while the core is away.
    /// Held as bus addresses; a core masks to its own width.
    breakpoints: BTreeSet<u32>,
    /// UI copy of the watchpoint list, kept editable while the core is away.
    watchpoints: Vec<Watch>,
    /// Lightweight status published every frame while the core is away; feeds
    /// the sidebar summary until the first full snapshot lands.
    last_status: Option<RunningStatus>,
    /// The per-vblank inspection snapshot the running panes render from.
    /// Boxed — a snapshot carries a full VRAM copy and shouldn't inflate the
    /// paused-path `Debugger` (and the `Game` enum) by that much.
    last_snapshot: Option<DebugView>,
    /// The memory viewer's interest window as of the last vblank, fed by the
    /// emu thread while the core runs; drives the running memory browser.
    last_memory_window: Option<MemoryWindow>,
    /// The core's static region map, cached at build so the memory pane's
    /// running browser and jump-to-address work while the core is away.
    memory_regions: &'static [MemoryRegion],
    sidebar: Sidebar,
    panes: DebuggerPanes,
    running: bool,
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

/// A console handed back by a system with no debugger backend, with the
/// screen view that was to be carried over.
pub type ReturnedConsole = Box<(Box<dyn SystemConsole>, ScreenView)>;

/// The panes a platform presents. Every platform whose debugger this build can
/// construct registers a family behind the same feature gate.
fn pane_family(platform: Platform) -> &'static panes::Family {
    panes::family_for(platform).expect("a debugger's platform registers panes")
}

impl Debugger {
    pub fn new(
        console: Box<dyn SystemConsole>,
        platform: Platform,
    ) -> Result<Self, Box<dyn SystemConsole>> {
        console.into_debugger().map(|core| {
            let panes = DebuggerPanes::new(pane_family(platform));
            Self::build(core, panes, platform)
        })
    }

    pub fn from_console(
        console: Box<dyn SystemConsole>,
        screen_view: ScreenView,
        platform: Platform,
    ) -> Result<Self, ReturnedConsole> {
        match console.into_debugger() {
            Ok(core) => {
                let panes = DebuggerPanes::with_screen(pane_family(platform), screen_view);
                Ok(Self::build(core, panes, platform))
            }
            Err(console) => Err(Box::new((console, screen_view))),
        }
    }

    fn build(core: Box<dyn SystemDebugger>, panes: DebuggerPanes, platform: Platform) -> Self {
        let memory_regions = core.memory_regions();
        Self {
            debugger: Some(core),
            rom_path: None,
            platform,
            breakpoints: BTreeSet::new(),
            watchpoints: Vec::new(),
            last_status: None,
            last_snapshot: None,
            last_memory_window: None,
            memory_regions,
            sidebar: Sidebar::new(),
            panes,
            running: false,
            frame: 0,
            bottom_panes: None,
            bottom_handles: HashMap::new(),
            main_split: None,
            breakpoint_input: String::new(),
            watchpoint_input: String::new(),
            watchpoint_kind: AccessKind::Write,
            label_address_input: String::new(),
            label_name_input: String::new(),
        }
    }

    /// Load the ROM's debug sidecars, whatever the system keeps beside its
    /// media. No-op while the core is away on the emu thread.
    pub fn load_sidecars(&mut self, rom_path: &std::path::Path) {
        if let Some(core) = &mut self.debugger {
            core.load_sidecars(rom_path);
            self.rom_path = Some(rom_path.to_path_buf());
        }
    }

    fn save_sidecars(&self) {
        if let (Some(core), Some(rom_path)) = (&self.debugger, &self.rom_path) {
            core.save_sidecars(rom_path);
        }
    }

    /// Save contents, available only while the core is on the UI thread.
    pub fn battery_save(&self) -> Option<Vec<u8>> {
        self.debugger.as_ref().and_then(|core| core.battery_save())
    }

    /// Game title, available only while the core is on the UI thread.
    pub fn game_title(&self) -> Option<String> {
        self.debugger.as_ref().map(|core| core.game_title())
    }

    pub fn capture_screenshot(&self, options: &CaptureOptions) -> Option<FrameCapture> {
        self.debugger
            .as_ref()
            .map(|core| FrameCapture::from_frame(&core.screen_display(), options))
    }

    /// Take the core to hand it to the emu thread for running.
    pub fn take_payload(&mut self) -> Option<DebuggerPayload> {
        let frame = self.frame;
        self.debugger
            .take()
            .map(|core| DebuggerPayload { core, frame })
    }

    /// Put the core back when the emu thread returns it on pause or breakpoint.
    pub fn restore_payload(&mut self, payload: DebuggerPayload) {
        let mut core = payload.core;
        // Resync from the UI's set: a breakpoint edit can race the payload's
        // return and get dropped by the idle emu thread.
        let stale: Vec<u32> = core
            .breakpoints()
            .difference(&self.breakpoints)
            .copied()
            .collect();
        for address in stale {
            core.clear_breakpoint(address);
        }
        for &address in &self.breakpoints {
            core.set_breakpoint(address);
        }
        let stale: Vec<Watch> = core
            .watches()
            .into_iter()
            .filter(|w| !self.watchpoints.contains(w))
            .collect();
        for watch in &stale {
            core.remove_watch(watch);
        }
        for watch in &self.watchpoints {
            core.add_watch(watch.clone());
        }
        self.debugger = Some(core);
        self.frame = payload.frame;
        self.last_status = None;
        self.last_snapshot = None;
        self.last_memory_window = None;
    }

    /// Whether the core is away on the emu thread.
    pub fn is_detached(&self) -> bool {
        self.debugger.is_none()
    }

    /// Update the screen pane from the emu thread's latest-frame slot.
    pub fn apply_frame(&mut self, display: Frame) {
        self.panes
            .update(panes::Message::Pane(panes::PaneMessage::Screen(
                screen::Message::Update(std::sync::Arc::new(display)),
            )));
    }

    /// Update the live status shown while the core runs on the emu thread.
    pub fn apply_status(&mut self, status: RunningStatus) {
        self.frame = status.frame;
        self.last_status = Some(status);
    }

    /// Update the per-vblank inspection snapshot the running panes render from.
    pub fn apply_snapshot(&mut self, view: DebugView) {
        self.frame = view.frame();
        self.last_snapshot = Some(view);
    }

    /// Update the memory viewer's interest window from the emu thread's slot.
    pub fn apply_memory_window(&mut self, window: MemoryWindow) {
        self.last_memory_window = Some(window);
    }

    /// The span the memory pane wants peeked while running: its current view,
    /// resolved against the cached region map. `None` when no memory pane is
    /// shown or the family has no region map.
    pub fn memory_interest(&self) -> Option<memory::MemoryInterest> {
        memory::interest_for(self.memory_regions, self.panes.memory_selection()?)
    }

    /// Whether any consumer wants per-channel waveform capture on — currently
    /// the audio scope pane. Off frees the core's capture rings.
    pub fn wants_wave_capture(&self) -> bool {
        self.panes.plane_shown(panes::DebuggerPane::Audio)
    }

    /// Whether any consumer wants per-vblank graphics capture on — the tile,
    /// tile-map, and sprite panes. Off drops the snapshot's VRAM clone and its
    /// surface decode.
    pub fn wants_graphics_capture(&self) -> bool {
        self.panes.plane_shown(panes::DebuggerPane::Tiles)
            || self
                .panes
                .plane_shown(panes::DebuggerPane::TileMap(TileMapId(0)))
            || self
                .panes
                .plane_shown(panes::DebuggerPane::TileMap(TileMapId(1)))
            || self.panes.plane_shown(panes::DebuggerPane::Sprites)
    }

    pub fn audio_coupling(&self) -> Option<missingno_core::HighPass> {
        self.debugger
            .as_ref()
            .and_then(|core| core.audio_coupling())
    }

    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        match &mut self.debugger {
            Some(core) => core.drain_audio_samples(),
            None => Vec::new(),
        }
    }

    pub fn disable_debugger(mut self, use_sgb_colors: bool, frame_blending: bool) -> Emulator {
        self.save_sidecars();
        let core = self
            .debugger
            .take()
            .expect("core present when disabling the debugger");
        let screen_view = self.panes.take_screen_view();
        Emulator::from_debugger(
            core.into_console(),
            screen_view,
            self.platform,
            use_sgb_colors,
            frame_blending,
        )
    }

    fn set_breakpoint(&mut self, address: u32, emu: Option<&EmuHandle>) {
        self.breakpoints.insert(address);
        match &mut self.debugger {
            Some(core) => core.set_breakpoint(address),
            None => {
                if let Some(emu) = emu {
                    emu.send(EmuCommand::SetBreakpoint(address));
                }
            }
        }
    }

    fn clear_breakpoint(&mut self, address: u32, emu: Option<&EmuHandle>) {
        self.breakpoints.remove(&address);
        match &mut self.debugger {
            Some(core) => core.clear_breakpoint(address),
            None => {
                if let Some(emu) = emu {
                    emu.send(EmuCommand::ClearBreakpoint(address));
                }
            }
        }
    }

    fn add_watchpoint(&mut self, watch: Watch, emu: Option<&EmuHandle>) {
        if self.watchpoints.contains(&watch) {
            return;
        }
        self.watchpoints.push(watch.clone());
        match &mut self.debugger {
            Some(core) => core.add_watch(watch),
            None => {
                if let Some(emu) = emu {
                    emu.send(EmuCommand::AddWatchpoint(watch));
                }
            }
        }
    }

    fn remove_watchpoint(&mut self, watch: &Watch, emu: Option<&EmuHandle>) {
        self.watchpoints.retain(|w| w != watch);
        match &mut self.debugger {
            Some(core) => core.remove_watch(watch),
            None => {
                if let Some(emu) = emu {
                    emu.send(EmuCommand::RemoveWatchpoint(watch.clone()));
                }
            }
        }
    }

    /// The most recent watch the core stopped on, present only while the
    /// core is on the UI thread (paused after a hit).
    fn last_watchpoint_hit(&self) -> Option<Watch> {
        self.debugger
            .as_ref()
            .and_then(|core| core.last_watch_hit())
    }

    fn display_task(display: Option<Frame>) -> Task<app::Message> {
        match display {
            Some(display) => {
                Task::done(screen::Message::Update(std::sync::Arc::new(display)).into())
            }
            None => Task::none(),
        }
    }

    pub fn update(&mut self, message: Message, emu: Option<&EmuHandle>) -> Task<app::Message> {
        match message {
            Message::Step => {
                let Some(core) = &mut self.debugger else {
                    return Task::none();
                };
                Self::display_task(core.step().into_frame())
            }
            Message::StepOver => {
                let Some(core) = &mut self.debugger else {
                    return Task::none();
                };
                Self::display_task(core.step_over().into_frame())
            }
            Message::StepFrame => {
                let Some(core) = &mut self.debugger else {
                    return Task::none();
                };
                let outcome = core.step_frame();
                self.frame += 1;
                if matches!(
                    outcome,
                    StepOutcome::Breakpoint { .. } | StepOutcome::WatchHit(_)
                ) {
                    self.running = false;
                }
                Self::display_task(outcome.into_frame())
            }
            Message::CaptureFrame => {
                let Some(core) = &self.debugger else {
                    return Task::none();
                };
                let title = core.game_title().to_lowercase().replace(' ', "_");
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
                let Some(core) = &mut self.debugger else {
                    return Task::none();
                };
                match core.capture_trace(&path) {
                    Some(display) => {
                        self.frame += 1;
                        Self::display_task(Some(display))
                    }
                    None => Task::none(),
                }
            }

            Message::SetBreakpoint(address) => {
                self.set_breakpoint(address, emu);
                Task::none()
            }
            Message::ClearBreakpoint(address) => {
                self.clear_breakpoint(address, emu);
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
                    self.set_breakpoint(address as u32, emu);
                    self.breakpoint_input.clear();
                }
                Task::none()
            }

            Message::RemoveWatchpoint(watch) => {
                self.remove_watchpoint(&watch, emu);
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
                    self.add_watchpoint(Watch::single(key, Some(address as u32), None), emu);
                    self.watchpoint_input.clear();
                }
                Task::none()
            }

            Message::RemoveLabel(symbol) => {
                if let Some(core) = &mut self.debugger {
                    core.remove_symbol(&symbol);
                }
                self.save_sidecars();
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
                    if let Some(core) = &mut self.debugger {
                        core.add_symbol(address as u32, std::mem::take(&mut self.label_name_input));
                        self.label_address_input.clear();
                    }
                    self.save_sidecars();
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

            Message::Pane(message) => {
                // Keep the memory pane's region cache fresh so its
                // jump-to-address resolves while the core runs on the emu
                // thread; harmless no-op for every other pane.
                self.panes
                    .update(panes::Message::Pane(panes::PaneMessage::Memory(
                        memory::Message::SetRegions(self.memory_regions),
                    )));
                self.panes.update(message);
                // Opening or closing a pane can change what the running core
                // must produce: re-aim the vblank memory peek and toggle
                // waveform capture. While detached both ride the emu thread;
                // while the core is here, capture toggles on it directly so the
                // paused tail fills as the user steps.
                let wants_waves = self.wants_wave_capture();
                let wants_graphics = self.wants_graphics_capture();
                if self.is_detached() {
                    if let Some(emu) = emu {
                        emu.send(EmuCommand::SetMemoryInterest(self.memory_interest()));
                        emu.set_wave_capture(wants_waves);
                        emu.set_graphics_capture(wants_graphics);
                    }
                } else if let Some(core) = &mut self.debugger {
                    core.set_wave_capture(wants_waves);
                    core.set_graphics_capture(wants_graphics);
                }
                Task::none()
            }
        }
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.panes.set_palette(palette);
    }

    pub fn set_frame_blending(&mut self, blend: bool) {
        self.panes.set_frame_blending(blend);
    }

    pub fn view(&self) -> Element<'_, app::Message> {
        let Some(core) = &self.debugger else {
            return self.running_view();
        };
        let family_any = core.family_state();
        let gb_source = inspect::as_inspect_source(family_any);
        let colors = gb_source.map(|source| source.colors(self.panes.palette()));
        let gb = colors.as_ref().map(|colors| GbPaneContext { colors });
        let readout = self
            .panes
            .memory_selection()
            .map(|selection| memory::build_readout(core.as_ref(), selection));
        let disasm_readout = self
            .panes
            .plane_shown(panes::DebuggerPane::Disassembly)
            .then(|| disassembly::paused_readout(core.as_ref()));
        // The frozen tail the core still holds while paused; `None` unless the
        // audio scope has capture on.
        let waves = core.channel_waves();
        // The decoded surfaces the core still holds while paused; `None` unless
        // a graphics pane has capture on.
        let graphics = core.graphics();
        let ctx = PaneContext {
            gb,
            family: family_any,
            breakpoints: &self.breakpoints,
            memory: readout.as_ref().map(memory::MemoryPaneData::paused),
            disasm: disasm_readout
                .as_ref()
                .map(disassembly::DisasmPaneData::new),
            waves: waves.as_deref(),
            graphics: graphics.as_ref(),
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

        let sidebar = self.sidebar.view(core.sidebar_sections(), colors.as_ref());
        row![sidebar, center, self.icon_rail(),]
            .spacing(s())
            .padding(s())
            .into()
    }

    /// The view while the core runs on the emu thread. The screen pane stays
    /// live from the frame slot; every other pane and the sidebar render from
    /// the per-vblank inspection snapshot, falling back to titled placeholders
    /// and the [`RunningStatus`] summary until the first snapshot arrives.
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
        let family_any = snapshot.family_state();
        // The colour context is a Game Boy pane surface only when the snapshot
        // is a GB-family one; other families leave `gb` empty.
        let is_gb_family =
            family_any.is::<inspect::GbSnapshot>() || family_any.is::<inspect::CgbSnapshot>();
        let gb = colors
            .filter(|_| is_gb_family)
            .map(|colors| GbPaneContext { colors });
        let disasm_readout = self
            .panes
            .plane_shown(panes::DebuggerPane::Disassembly)
            .then(|| disassembly::running_readout(snapshot))
            .flatten();
        // This vblank's captured windows; `None` unless capture is on.
        let waves = snapshot.channel_waves();
        // This vblank's decoded surfaces; `None` unless graphics capture is on.
        let graphics = snapshot.graphics();
        self.panes.view(Some(PaneContext {
            gb,
            family: family_any,
            breakpoints: &self.breakpoints,
            memory: self.running_memory(snapshot),
            disasm: disasm_readout
                .as_ref()
                .map(disassembly::DisasmPaneData::new),
            waves: waves.as_deref(),
            graphics: graphics.as_ref(),
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
                self.memory_regions,
                self.last_memory_window.as_ref(),
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

            panes::pane(panes::title_bar(panel.label()), content)
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

        let panel = match self.last_watchpoint_hit() {
            Some(hit) => column![
                text(format!("hit: {}", watch_summary(&hit))).font(fonts::monospace()),
                watchpoint_list,
                add_row,
            ],
            None => column![watchpoint_list, add_row],
        };

        panel.spacing(s()).padding(s()).into()
    }

    /// Label editing needs the core on the UI thread; while it runs on the
    /// emu thread the panel is read-only.
    fn labels_content(&self) -> Element<'_, app::Message> {
        let Some(core) = &self.debugger else {
            return column![text("Pause to edit labels").font(fonts::monospace()),]
                .spacing(s())
                .padding(s())
                .into();
        };
        let symbols = core.symbols();

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
            rail_icon(
                pane.icon(),
                &pane.to_string(),
                self.panes.plane_shown(pane),
                panes::Message::if_shown(pane, self.panes.plane_shown(pane)).into(),
            )
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

    pub fn running(&self) -> bool {
        self.running
    }

    pub fn run(&mut self) {
        self.running = true;
    }

    pub fn pause(&mut self) {
        self.running = false;
    }

    pub fn reset(&mut self) {
        if let Some(core) = &mut self.debugger {
            core.reset();
            self.frame = 0;
        }
    }

    pub fn set_control(&mut self, control: ControlId, input: ControlInput) {
        if let Some(core) = &mut self.debugger {
            core.set_control(control, input);
        }
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
    use crate::app::debugger::sidebar::tooltip_style;
    use iced::widget::tooltip;

    let color = if active {
        palette::PURPLE
    } else {
        palette::SURFACE2
    };

    let btn: Element<'_, app::Message> = button(icons::m_colored(icon, color))
        .on_press(message)
        .style(button::text)
        .into();

    tooltip(
        btn,
        container(text(label.to_owned()).font(fonts::monospace()).size(13.0)).padding([2.0, s()]),
        tooltip::Position::Left,
    )
    .style(tooltip_style)
    .into()
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
