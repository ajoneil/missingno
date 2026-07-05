use std::collections::{BTreeSet, HashMap};

use iced::{
    Element, Length, Task,
    alignment::Vertical,
    widget::{Column, button, column, container, pane_grid, row, text, text_input},
};

use crate::app::{
    self,
    console::{AnyConsole, ConsoleUi},
    emu_thread::{EmuCommand, EmuHandle, RunningStatus},
    emulator::Emulator,
    library::activity::FrameCapture,
    screen::{ScreenDisplay, ScreenView},
    ui::{
        fonts, icons, palette,
        sizes::{s, xs},
    },
};
use missingno_gb::{
    cartridge::Cartridge, joypad::Button, ppu::rendering::Mode, ppu::types::palette::PaletteChoice,
};

use panes::DebuggerPanes;
use sidebar::Sidebar;

mod audio;
mod instructions;
mod interrupts;
pub mod panes;
mod ppu;
mod screen;
mod sidebar;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BottomPanel {
    Breakpoints,
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

    BottomPane(BottomPaneMessage),
    MainSplitResize(pane_grid::ResizeEvent),

    Sidebar(sidebar::Message),
    Pane(panes::Message),
}

impl Into<super::Message> for Message {
    fn into(self) -> super::Message {
        super::Message::Debugger(self)
    }
}

/// The wrapped console's debugger, dispatched to the matching [`Debugger<M>`].
pub enum AnyDebugger {
    Dmg(Debugger<missingno_gb::Dmg>),
    Cgb(Debugger<missingno_gbc::Cgb>),
}

impl AnyDebugger {
    pub fn new(console: AnyConsole) -> Self {
        match console {
            AnyConsole::Dmg(game_boy) => Self::Dmg(Debugger::new(game_boy)),
            AnyConsole::Cgb(console) => Self::Cgb(Debugger::new(console)),
        }
    }

    pub fn from_emulator(console: AnyConsole, screen_view: ScreenView) -> Self {
        match console {
            AnyConsole::Dmg(game_boy) => Self::Dmg(Debugger::from_console(game_boy, screen_view)),
            AnyConsole::Cgb(console) => Self::Cgb(Debugger::from_console(console, screen_view)),
        }
    }

    pub fn disable_debugger(self, use_sgb_colors: bool) -> Emulator {
        match self {
            Self::Dmg(debugger) => debugger.into_emulator(use_sgb_colors),
            Self::Cgb(debugger) => debugger.into_emulator(use_sgb_colors),
        }
    }

    pub fn update(&mut self, message: Message, emu: Option<&EmuHandle>) -> Task<app::Message> {
        match self {
            Self::Dmg(debugger) => debugger.update(message, emu),
            Self::Cgb(debugger) => debugger.update(message, emu),
        }
    }

    pub fn view(&self) -> Element<'_, app::Message> {
        match self {
            Self::Dmg(debugger) => debugger.view(),
            Self::Cgb(debugger) => debugger.view(),
        }
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        match self {
            Self::Dmg(debugger) => debugger.set_palette(palette),
            Self::Cgb(debugger) => debugger.set_palette(palette),
        }
    }

    /// The cartridge, present only while the core is on the UI thread.
    pub fn cartridge(&self) -> Option<&Cartridge> {
        match self {
            Self::Dmg(debugger) => debugger.game_boy().map(|gb| gb.cartridge()),
            Self::Cgb(debugger) => debugger.game_boy().map(|gb| gb.cartridge()),
        }
    }

    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        match self {
            Self::Dmg(debugger) => debugger.drain_audio_samples(),
            Self::Cgb(debugger) => debugger.drain_audio_samples(),
        }
    }

    pub fn capture_screenshot(
        &self,
        use_sgb_colors: bool,
        palette_name: &str,
    ) -> Option<FrameCapture> {
        match self {
            Self::Dmg(debugger) => debugger
                .game_boy()
                .map(|gb| missingno_gb::Dmg::capture_frame(gb, use_sgb_colors, palette_name)),
            Self::Cgb(debugger) => debugger
                .game_boy()
                .map(|gb| missingno_gbc::Cgb::capture_frame(gb, use_sgb_colors, palette_name)),
        }
    }

    /// Take the core to hand it to the emu thread for running.
    pub fn take_payload(&mut self) -> Option<DebuggerPayload> {
        match self {
            Self::Dmg(debugger) => debugger.take_core().map(|(core, frame)| DebuggerPayload {
                core: DebuggerCore::Dmg(Box::new(core)),
                frame,
            }),
            Self::Cgb(debugger) => debugger.take_core().map(|(core, frame)| DebuggerPayload {
                core: DebuggerCore::Cgb(Box::new(core)),
                frame,
            }),
        }
    }

    /// Put the core back when the emu thread returns it on pause or breakpoint.
    pub fn restore_payload(&mut self, payload: DebuggerPayload) {
        match (self, payload.core) {
            (Self::Dmg(debugger), DebuggerCore::Dmg(core)) => {
                debugger.restore_core(*core, payload.frame)
            }
            (Self::Cgb(debugger), DebuggerCore::Cgb(core)) => {
                debugger.restore_core(*core, payload.frame)
            }
            _ => {}
        }
    }

    /// Whether the core is away on the emu thread.
    pub fn is_detached(&self) -> bool {
        match self {
            Self::Dmg(debugger) => debugger.debugger.is_none(),
            Self::Cgb(debugger) => debugger.debugger.is_none(),
        }
    }

    /// Update the screen pane from the emu thread's latest-frame slot.
    pub fn apply_frame(&mut self, display: ScreenDisplay) {
        match self {
            Self::Dmg(debugger) => debugger.apply_frame(display),
            Self::Cgb(debugger) => debugger.apply_frame(display),
        }
    }

    /// Update the live status shown while the core runs on the emu thread.
    pub fn apply_status(&mut self, status: RunningStatus) {
        match self {
            Self::Dmg(debugger) => debugger.apply_status(status),
            Self::Cgb(debugger) => debugger.apply_status(status),
        }
    }

    pub fn running(&self) -> bool {
        match self {
            Self::Dmg(debugger) => debugger.running(),
            Self::Cgb(debugger) => debugger.running(),
        }
    }

    pub fn run(&mut self) {
        match self {
            Self::Dmg(debugger) => debugger.run(),
            Self::Cgb(debugger) => debugger.run(),
        }
    }

    pub fn pause(&mut self) {
        match self {
            Self::Dmg(debugger) => debugger.pause(),
            Self::Cgb(debugger) => debugger.pause(),
        }
    }

    pub fn reset(&mut self) {
        match self {
            Self::Dmg(debugger) => debugger.reset(),
            Self::Cgb(debugger) => debugger.reset(),
        }
    }

    pub fn press_button(&mut self, button: Button) {
        match self {
            Self::Dmg(debugger) => debugger.press_button(button),
            Self::Cgb(debugger) => debugger.press_button(button),
        }
    }

    pub fn release_button(&mut self, button: Button) {
        match self {
            Self::Dmg(debugger) => debugger.release_button(button),
            Self::Cgb(debugger) => debugger.release_button(button),
        }
    }
}

/// The debugger state that moves to the emu thread while running: the core
/// (console, breakpoints, counters) plus the UI's frame counter. Pane and
/// layout state stays behind on the UI thread.
pub struct DebuggerPayload {
    core: DebuggerCore,
    frame: u64,
}

enum DebuggerCore {
    Dmg(Box<missingno_gb::debugger::Debugger<missingno_gb::Dmg>>),
    Cgb(Box<missingno_gb::debugger::Debugger<missingno_gbc::Cgb>>),
}

impl DebuggerPayload {
    /// Step until the next frame or breakpoint. Returns the display (if a
    /// frame completed) and whether a breakpoint stopped the run.
    pub fn step_frame(&mut self) -> (Option<ScreenDisplay>, bool) {
        self.frame += 1;
        match &mut self.core {
            DebuggerCore::Dmg(core) => step_core_frame(core),
            DebuggerCore::Cgb(core) => step_core_frame(core),
        }
    }

    pub fn running_status(&self) -> RunningStatus {
        let (pc, sp, ly, mode) = match &self.core {
            DebuggerCore::Dmg(core) => console_status(core.game_boy()),
            DebuggerCore::Cgb(core) => console_status(core.game_boy()),
        };
        RunningStatus {
            pc,
            sp,
            ly,
            mode,
            frame: self.frame,
        }
    }

    pub fn reset(&mut self) {
        self.frame = 0;
        match &mut self.core {
            DebuggerCore::Dmg(core) => core.reset(),
            DebuggerCore::Cgb(core) => core.reset(),
        }
    }

    pub fn press_button(&mut self, button: Button) {
        match &mut self.core {
            DebuggerCore::Dmg(core) => core.game_boy_mut().press_button(button),
            DebuggerCore::Cgb(core) => core.game_boy_mut().press_button(button),
        }
    }

    pub fn release_button(&mut self, button: Button) {
        match &mut self.core {
            DebuggerCore::Dmg(core) => core.game_boy_mut().release_button(button),
            DebuggerCore::Cgb(core) => core.game_boy_mut().release_button(button),
        }
    }

    pub fn set_breakpoint(&mut self, address: u16) {
        match &mut self.core {
            DebuggerCore::Dmg(core) => core.set_breakpoint(address),
            DebuggerCore::Cgb(core) => core.set_breakpoint(address),
        }
    }

    pub fn clear_breakpoint(&mut self, address: u16) {
        match &mut self.core {
            DebuggerCore::Dmg(core) => core.clear_breakpoint(address),
            DebuggerCore::Cgb(core) => core.clear_breakpoint(address),
        }
    }

    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        match &mut self.core {
            DebuggerCore::Dmg(core) => core.game_boy_mut().drain_audio_samples(),
            DebuggerCore::Cgb(core) => core.game_boy_mut().drain_audio_samples(),
        }
    }

    pub fn capture_frame(&self, use_sgb_colors: bool, palette_name: &str) -> FrameCapture {
        match &self.core {
            DebuggerCore::Dmg(core) => {
                missingno_gb::Dmg::capture_frame(core.game_boy(), use_sgb_colors, palette_name)
            }
            DebuggerCore::Cgb(core) => {
                missingno_gbc::Cgb::capture_frame(core.game_boy(), use_sgb_colors, palette_name)
            }
        }
    }

    pub fn cartridge(&self) -> &Cartridge {
        match &self.core {
            DebuggerCore::Dmg(core) => core.game_boy().cartridge(),
            DebuggerCore::Cgb(core) => core.game_boy().cartridge(),
        }
    }
}

fn step_core_frame<M: ConsoleUi>(
    core: &mut missingno_gb::debugger::Debugger<M>,
) -> (Option<ScreenDisplay>, bool) {
    let screen = core.step_frame();
    let breakpoint_hit = screen.is_none();
    (M::screen_display(core.game_boy(), screen), breakpoint_hit)
}

fn console_status<M: ConsoleUi>(console: &missingno_gb::Console<M>) -> (u16, u16, u8, Mode) {
    (
        console.cpu().ir_address,
        console.cpu().stack_pointer,
        console.ppu().video.ly(),
        console.ppu().mode(),
    )
}

pub struct Debugger<M: ConsoleUi> {
    /// The core (console + breakpoints) — `None` while it runs on the emu thread.
    debugger: Option<missingno_gb::debugger::Debugger<M>>,
    /// UI copy of the breakpoint set, kept editable while the core is away.
    breakpoints: BTreeSet<u16>,
    /// Live state published by the emu thread while the core is away.
    last_status: Option<RunningStatus>,
    sidebar: Sidebar,
    panes: DebuggerPanes,
    running: bool,
    frame: u64,
    bottom_panes: Option<pane_grid::State<BottomPanel>>,
    bottom_handles: HashMap<BottomPanel, pane_grid::Pane>,
    main_split: Option<pane_grid::State<MainSplit>>,
    breakpoint_input: String,
}

impl<M: ConsoleUi> Debugger<M> {
    pub fn new(console: missingno_gb::Console<M>) -> Self {
        Self::build(console, DebuggerPanes::new())
    }

    pub fn from_console(console: missingno_gb::Console<M>, screen_view: ScreenView) -> Self {
        Self::build(console, DebuggerPanes::with_screen(screen_view))
    }

    fn build(console: missingno_gb::Console<M>, panes: DebuggerPanes) -> Self {
        Self {
            debugger: Some(missingno_gb::debugger::Debugger::new(console)),
            breakpoints: BTreeSet::new(),
            last_status: None,
            sidebar: Sidebar::new(),
            panes,
            running: false,
            frame: 0,
            bottom_panes: None,
            bottom_handles: HashMap::new(),
            main_split: None,
            breakpoint_input: String::new(),
        }
    }

    /// The console, present only while the core is on the UI thread.
    pub fn game_boy(&self) -> Option<&missingno_gb::Console<M>> {
        self.debugger.as_ref().map(|core| core.game_boy())
    }

    fn take_core(&mut self) -> Option<(missingno_gb::debugger::Debugger<M>, u64)> {
        let frame = self.frame;
        self.debugger.take().map(|core| (core, frame))
    }

    fn restore_core(&mut self, mut core: missingno_gb::debugger::Debugger<M>, frame: u64) {
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
        self.debugger = Some(core);
        self.frame = frame;
        self.last_status = None;
    }

    fn apply_frame(&mut self, display: ScreenDisplay) {
        self.panes
            .update(panes::Message::Pane(panes::PaneMessage::Screen(
                screen::Message::Update(display),
            )));
    }

    fn apply_status(&mut self, status: RunningStatus) {
        self.frame = status.frame;
        self.last_status = Some(status);
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        match &mut self.debugger {
            Some(core) => core.game_boy_mut().drain_audio_samples(),
            None => Vec::new(),
        }
    }

    fn into_emulator(self, use_sgb_colors: bool) -> Emulator
    where
        AnyConsole: From<missingno_gb::Console<M>>,
    {
        let core = self
            .debugger
            .expect("core present when disabling the debugger");
        let screen_view = self.panes.take_screen_view();
        Emulator::from_debugger(core.game_boy_take().into(), screen_view, use_sgb_colors)
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

    fn screen_update_task(&self, screen: Option<M::Screen>) -> Task<app::Message> {
        let Some(core) = &self.debugger else {
            return Task::none();
        };
        match M::screen_display(core.game_boy(), screen) {
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
                let screen = core.step();
                self.screen_update_task(screen)
            }
            Message::StepOver => {
                let Some(core) = &mut self.debugger else {
                    return Task::none();
                };
                let screen = core.step_over();
                self.screen_update_task(screen)
            }
            Message::StepFrame => {
                let Some(core) = &mut self.debugger else {
                    return Task::none();
                };
                let screen = core.step_frame();
                self.frame += 1;
                if screen.is_none() {
                    self.running = false;
                }
                self.screen_update_task(screen)
            }
            Message::CaptureFrame => {
                let Some(core) = &self.debugger else {
                    return Task::none();
                };
                let title = core
                    .game_boy()
                    .cartridge()
                    .title()
                    .to_lowercase()
                    .replace(' ', "_");
                let default_name = format!("{title}_frame{}.gbtrace", self.frame);

                let dialog = rfd::AsyncFileDialog::new()
                    .set_file_name(&default_name)
                    .add_filter("gbtrace", &["gbtrace"]);

                return Task::perform(dialog.save_file(), |handle| match handle {
                    Some(h) => Message::CaptureFrameTo(h.path().to_path_buf()).into(),
                    None => app::Message::None,
                });
            }
            Message::CaptureFrameTo(path) => {
                let Some(core) = &mut self.debugger else {
                    return Task::none();
                };
                match core.capture_frame(&path) {
                    Ok(screen) => {
                        self.frame += 1;
                        self.screen_update_task(Some(screen))
                    }
                    Err(_) => Task::none(),
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
                        if let pane_grid::DragEvent::Dropped { pane, target } = drag {
                            if let Some(panes) = &mut self.bottom_panes {
                                panes.drop(pane, target);
                            }
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

    pub fn view(&self) -> Element<'_, app::Message> {
        let Some(core) = &self.debugger else {
            return self.running_view();
        };
        let colors = M::colors(core.game_boy(), self.panes.palette());

        let center: Element<'_, app::Message> = if let Some(split_state) = &self.main_split {
            pane_grid(split_state, |_handle, zone, _maximized| {
                let content: Element<'_, app::Message> = match zone {
                    MainSplit::Top => self.panes.view(core, &colors),
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
            self.panes.view(core, &colors)
        };

        row![self.sidebar.view(core, &colors), center, self.icon_rail(),]
            .spacing(s())
            .padding(s())
            .into()
    }

    /// The view while the core runs on the emu thread: the screen pane stays
    /// live from the frame slot, deep-inspection panes show placeholders, and
    /// the sidebar summarises the published [`RunningStatus`].
    fn running_view(&self) -> Element<'_, app::Message> {
        let center: Element<'_, app::Message> = if let Some(split_state) = &self.main_split {
            pane_grid(split_state, |_handle, zone, _maximized| {
                let content: Element<'_, app::Message> = match zone {
                    MainSplit::Top => self.panes.running_view(),
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
            self.panes.running_view()
        };

        row![
            self.sidebar.running_view(self.last_status.as_ref()),
            center,
            self.icon_rail(),
        ]
        .spacing(s())
        .padding(s())
        .into()
    }

    fn bottom_pane_grid<'a>(
        &'a self,
        state: &'a pane_grid::State<BottomPanel>,
    ) -> Element<'a, app::Message> {
        pane_grid(state, |_handle, panel, _maximized| {
            let content: Element<'_, app::Message> = match panel {
                BottomPanel::Breakpoints => self.breakpoints_content(),
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

    fn icon_rail(&self) -> Element<'_, app::Message> {
        use icons::Icon;

        let pane_buttons = self.panes.available_panes().iter().map(|&pane| {
            rail_icon(
                pane.icon(),
                &pane.to_string(),
                self.panes.plane_shown(pane),
                panes::Message::if_shown(pane, self.panes.plane_shown(pane)).into(),
            )
        });

        let panel_buttons = [(BottomPanel::Breakpoints, Icon::Circle, "Breakpoints")]
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
            core.game_boy_mut().press_button(button);
        }
    }

    pub fn release_button(&mut self, button: Button) {
        if let Some(core) = &mut self.debugger {
            core.game_boy_mut().release_button(button);
        }
    }
}

impl BottomPanel {
    fn label(&self) -> &'static str {
        match self {
            BottomPanel::Breakpoints => "Breakpoints",
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
