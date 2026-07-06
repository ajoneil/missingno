use std::collections::{BTreeSet, HashMap};

use iced::{
    Element, Length, Task,
    alignment::Vertical,
    widget::{Column, button, column, container, pane_grid, pick_list, row, text, text_input},
};

use crate::app::{
    self,
    console::ConsoleColors,
    emu_thread::{DebuggerPayload, EmuCommand, EmuHandle, RunningStatus},
    emulator::Emulator,
    library::activity::FrameCapture,
    screen::{ScreenDisplay, ScreenView},
    system::{SystemConsole, SystemDebugger},
    ui::{
        fonts, icons, palette,
        sizes::{s, xs},
    },
};
use missingno_gb::{
    debugger::{WatchCondition, symbols::Symbol},
    joypad::Button,
    ppu::types::palette::PaletteChoice,
};

use inspect::{DebugView, InspectSource};
use panes::{DebuggerPanes, PaneContext};
use sidebar::Sidebar;

mod audio;
pub mod inspect;
mod instructions;
mod interrupts;
mod layout;
pub mod panes;
mod ppu;
mod screen;
mod sidebar;

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

    SetBreakpoint(u16),
    ClearBreakpoint(u16),
    BreakpointInputChanged(String),
    AddBreakpoint,

    RemoveWatchpoint(WatchCondition),
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
    /// UI copy of the breakpoint set, kept editable while the core is away.
    breakpoints: BTreeSet<u16>,
    /// UI copy of the watchpoint list, kept editable while the core is away.
    watchpoints: Vec<WatchCondition>,
    /// Lightweight status published every frame while the core is away; feeds
    /// the sidebar summary until the first full snapshot lands.
    last_status: Option<RunningStatus>,
    /// The per-vblank inspection snapshot the running panes render from.
    /// Boxed — a snapshot carries a full VRAM copy and shouldn't inflate the
    /// paused-path `Debugger` (and the `Game` enum) by that much.
    last_snapshot: Option<DebugView>,
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

impl Debugger {
    pub fn new(console: Box<dyn SystemConsole>) -> Result<Self, Box<dyn SystemConsole>> {
        console
            .into_debugger()
            .map(|core| Self::build(core, DebuggerPanes::new()))
    }

    pub fn from_console(
        console: Box<dyn SystemConsole>,
        screen_view: ScreenView,
    ) -> Result<Self, ReturnedConsole> {
        match console.into_debugger() {
            Ok(core) => Ok(Self::build(core, DebuggerPanes::with_screen(screen_view))),
            Err(console) => Err(Box::new((console, screen_view))),
        }
    }

    fn build(core: Box<dyn SystemDebugger>, panes: DebuggerPanes) -> Self {
        Self {
            debugger: Some(core),
            rom_path: None,
            breakpoints: BTreeSet::new(),
            watchpoints: Vec::new(),
            last_status: None,
            last_snapshot: None,
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

    /// Load the ROM's debug sidecars — `.sym` labels and the `.cdl`
    /// code/data log. No-op while the core is away on the emu thread.
    pub fn load_symbols(&mut self, rom_path: &std::path::Path) {
        if let Some(core) = &mut self.debugger {
            core.set_symbols(missingno_gb::debugger::symbols::SymbolTable::for_rom(
                rom_path,
            ));
            core.load_cdl(&rom_path.with_extension("cdl"));
            self.rom_path = Some(rom_path.to_path_buf());
        }
    }

    fn save_sidecars(&self) {
        if let (Some(core), Some(rom_path)) = (&self.debugger, &self.rom_path) {
            core.save_cdl(&rom_path.with_extension("cdl"));
            core.save_symbols(&rom_path.with_extension("sym"));
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

    pub fn capture_screenshot(
        &self,
        use_sgb_colors: bool,
        palette_name: &str,
    ) -> Option<FrameCapture> {
        self.debugger
            .as_ref()
            .map(|core| core.capture_frame(use_sgb_colors, palette_name))
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
        let stale: Vec<u16> = core
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
        let stale: Vec<WatchCondition> = core
            .watchpoints()
            .iter()
            .filter(|w| !self.watchpoints.contains(w))
            .cloned()
            .collect();
        for condition in &stale {
            core.remove_watchpoint(condition);
        }
        for condition in &self.watchpoints {
            core.add_watchpoint(condition.clone());
        }
        self.debugger = Some(core);
        self.frame = payload.frame;
        self.last_status = None;
        self.last_snapshot = None;
    }

    /// Whether the core is away on the emu thread.
    pub fn is_detached(&self) -> bool {
        self.debugger.is_none()
    }

    /// Update the screen pane from the emu thread's latest-frame slot.
    pub fn apply_frame(&mut self, display: ScreenDisplay) {
        self.panes
            .update(panes::Message::Pane(panes::PaneMessage::Screen(
                screen::Message::Update(display),
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
            use_sgb_colors,
            frame_blending,
        )
    }

    fn set_breakpoint(&mut self, address: u16, emu: Option<&EmuHandle>) {
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

    fn clear_breakpoint(&mut self, address: u16, emu: Option<&EmuHandle>) {
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

    fn add_watchpoint(&mut self, condition: WatchCondition, emu: Option<&EmuHandle>) {
        if self.watchpoints.contains(&condition) {
            return;
        }
        self.watchpoints.push(condition.clone());
        match &mut self.debugger {
            Some(core) => core.add_watchpoint(condition),
            None => {
                if let Some(emu) = emu {
                    emu.send(EmuCommand::AddWatchpoint(condition));
                }
            }
        }
    }

    fn remove_watchpoint(&mut self, condition: &WatchCondition, emu: Option<&EmuHandle>) {
        self.watchpoints.retain(|w| w != condition);
        match &mut self.debugger {
            Some(core) => core.remove_watchpoint(condition),
            None => {
                if let Some(emu) = emu {
                    emu.send(EmuCommand::RemoveWatchpoint(condition.clone()));
                }
            }
        }
    }

    /// The most recent watchpoint the core stopped on, present only while the
    /// core is on the UI thread (paused after a hit).
    fn last_watchpoint_hit(&self) -> Option<WatchCondition> {
        self.debugger
            .as_ref()
            .and_then(|core| core.last_watchpoint_hit())
    }

    fn display_task(display: Option<ScreenDisplay>) -> Task<app::Message> {
        match display {
            Some(display) => Task::done(screen::Message::Update(display).into()),
            None => Task::none(),
        }
    }

    pub fn update(&mut self, message: Message, emu: Option<&EmuHandle>) -> Task<app::Message> {
        match message {
            Message::Step => {
                let Some(core) = &mut self.debugger else {
                    return Task::none();
                };
                Self::display_task(core.step())
            }
            Message::StepOver => {
                let Some(core) = &mut self.debugger else {
                    return Task::none();
                };
                Self::display_task(core.step_over())
            }
            Message::StepFrame => {
                let Some(core) = &mut self.debugger else {
                    return Task::none();
                };
                let (display, breakpoint_hit) = core.step_frame();
                self.frame += 1;
                if breakpoint_hit {
                    self.running = false;
                }
                Self::display_task(display)
            }
            Message::CaptureFrame => {
                let Some(core) = &self.debugger else {
                    return Task::none();
                };
                let title = core.game_title().to_lowercase().replace(' ', "_");
                let default_name = format!("{title}_frame{}.gbtrace", self.frame);

                let dialog = rfd::AsyncFileDialog::new()
                    .set_file_name(&default_name)
                    .add_filter("gbtrace", &["gbtrace"]);

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
                    self.set_breakpoint(address, emu);
                    self.breakpoint_input.clear();
                }
                Task::none()
            }

            Message::RemoveWatchpoint(condition) => {
                self.remove_watchpoint(&condition, emu);
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
                    let condition = match self.watchpoint_kind {
                        AccessKind::Read => WatchCondition::BusRead { address },
                        AccessKind::Write => WatchCondition::BusWrite { address },
                    };
                    self.add_watchpoint(condition, emu);
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
                        core.add_symbol(address, std::mem::take(&mut self.label_name_input));
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
                self.panes.update(message);
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
        let source: &dyn InspectSource = core.inspect();
        let colors = source.colors(self.panes.palette());
        let symbols = core.symbols();
        let cdl = core.cdl_window();

        let center: Element<'_, app::Message> = if let Some(split_state) = &self.main_split {
            pane_grid(split_state, |_handle, zone, _maximized| {
                let content: Element<'_, app::Message> = match zone {
                    MainSplit::Top => self.panes.view(Some(PaneContext {
                        source,
                        breakpoints: core.breakpoints(),
                        colors: &colors,
                        symbols: &symbols,
                        cdl: &cdl,
                    })),
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
            self.panes.view(Some(PaneContext {
                source,
                breakpoints: core.breakpoints(),
                colors: &colors,
                symbols: &symbols,
                cdl: &cdl,
            }))
        };

        row![self.sidebar.view(source, &colors), center, self.icon_rail(),]
            .spacing(s())
            .padding(s())
            .into()
    }

    /// The view while the core runs on the emu thread. The screen pane stays
    /// live from the frame slot; every other pane and the sidebar render from
    /// the per-vblank [`ConsoleSnapshot`], falling back to titled placeholders
    /// and the [`RunningStatus`] summary until the first snapshot arrives.
    fn running_view(&self) -> Element<'_, app::Message> {
        let colors = self
            .last_snapshot
            .as_deref()
            .map(|snapshot| snapshot.colors(self.panes.palette()));

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

        let sidebar = match (self.last_snapshot.as_deref(), &colors) {
            (Some(snapshot), Some(colors)) => self.sidebar.view(snapshot, colors),
            _ => self.sidebar.running_summary(self.last_status.as_ref()),
        };

        row![sidebar, center, self.icon_rail(),]
            .spacing(s())
            .padding(s())
            .into()
    }

    /// The main pane area while running: snapshot-backed panes when a snapshot
    /// is present, titled placeholders otherwise.
    fn running_center<'a>(&'a self, colors: Option<&ConsoleColors>) -> Element<'a, app::Message> {
        match (self.last_snapshot.as_deref(), colors) {
            (Some(snapshot), Some(colors)) => self.panes.view(Some(PaneContext {
                source: snapshot,
                breakpoints: &self.breakpoints,
                colors,
                symbols: snapshot.symbols(),
                cdl: snapshot.cdl(),
            })),
            _ => self.panes.view(None),
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
                text(format!("hit: {hit}")).font(fonts::monospace()),
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

    pub fn press_button(&mut self, button: Button) {
        if let Some(core) = &mut self.debugger {
            core.press_button(button);
        }
    }

    pub fn release_button(&mut self, button: Button) {
        if let Some(core) = &mut self.debugger {
            core.release_button(button);
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

fn watchpoint_row(condition: &WatchCondition) -> Element<'static, app::Message> {
    container(
        row![
            button(icons::m(icons::Icon::Close))
                .on_press(Message::RemoveWatchpoint(condition.clone()).into())
                .style(button::text),
            text(condition.to_string()).font(fonts::monospace())
        ]
        .align_y(Vertical::Center),
    )
    .into()
}

fn breakpoint_row(address: u16) -> Element<'static, app::Message> {
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
