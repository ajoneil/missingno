use std::collections::HashMap;
use std::time::Duration;

use iced::{
    Element, Length, Subscription, Task,
    alignment::Vertical,
    time,
    widget::{Column, button, column, container, pane_grid, row, text, text_input},
};

use crate::app::{
    self,
    console::{AnyConsole, ConsoleUi},
    emulator::Emulator,
    library::activity::FrameCapture,
    screen::ScreenView,
    ui::{
        fonts, icons, palette,
        sizes::{s, xs},
    },
};
use missingno_gb::{joypad::Button, ppu::types::palette::PaletteChoice};

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

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match self {
            Self::Dmg(debugger) => debugger.update(message),
            Self::Cgb(debugger) => debugger.update(message),
        }
    }

    pub fn view(&self) -> Element<'_, app::Message> {
        match self {
            Self::Dmg(debugger) => debugger.view(),
            Self::Cgb(debugger) => debugger.view(),
        }
    }

    pub fn subscription(&self) -> Subscription<app::Message> {
        match self {
            Self::Dmg(debugger) => debugger.subscription(),
            Self::Cgb(debugger) => debugger.subscription(),
        }
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        match self {
            Self::Dmg(debugger) => debugger.set_palette(palette),
            Self::Cgb(debugger) => debugger.set_palette(palette),
        }
    }

    pub fn cartridge(&self) -> &missingno_gb::cartridge::Cartridge {
        match self {
            Self::Dmg(debugger) => debugger.game_boy().cartridge(),
            Self::Cgb(debugger) => debugger.game_boy().cartridge(),
        }
    }

    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        match self {
            Self::Dmg(debugger) => debugger.drain_audio_samples(),
            Self::Cgb(debugger) => debugger.drain_audio_samples(),
        }
    }

    pub fn capture_screenshot(&self, use_sgb_colors: bool, palette_name: &str) -> FrameCapture {
        match self {
            Self::Dmg(debugger) => {
                missingno_gb::Dmg::capture_frame(debugger.game_boy(), use_sgb_colors, palette_name)
            }
            Self::Cgb(debugger) => {
                missingno_gbc::Cgb::capture_frame(debugger.game_boy(), use_sgb_colors, palette_name)
            }
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

pub struct Debugger<M: ConsoleUi> {
    debugger: missingno_gb::debugger::Debugger<M>,
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
        Self {
            debugger: missingno_gb::debugger::Debugger::new(console),
            sidebar: Sidebar::new(),
            panes: DebuggerPanes::new(),
            running: false,
            frame: 0,
            bottom_panes: None,
            bottom_handles: HashMap::new(),
            main_split: None,
            breakpoint_input: String::new(),
        }
    }

    pub fn from_console(console: missingno_gb::Console<M>, screen_view: ScreenView) -> Self {
        Self {
            debugger: missingno_gb::debugger::Debugger::new(console),
            sidebar: Sidebar::new(),
            panes: DebuggerPanes::with_screen(screen_view),
            running: false,
            frame: 0,
            bottom_panes: None,
            bottom_handles: HashMap::new(),
            main_split: None,
            breakpoint_input: String::new(),
        }
    }

    pub fn game_boy(&self) -> &missingno_gb::Console<M> {
        self.debugger.game_boy()
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.debugger.game_boy_mut().drain_audio_samples()
    }

    fn into_emulator(self, use_sgb_colors: bool) -> Emulator
    where
        AnyConsole: From<missingno_gb::Console<M>>,
    {
        let screen_view = self.panes.take_screen_view();
        Emulator::from_debugger(
            self.debugger.game_boy_take().into(),
            screen_view,
            use_sgb_colors,
        )
    }

    fn screen_update_task(&self, screen: Option<M::Screen>) -> Task<app::Message> {
        match M::screen_display(self.debugger.game_boy(), screen) {
            Some(display) => Task::done(screen::Message::Update(display).into()),
            None => Task::none(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
            Message::Step => {
                let screen = self.debugger.step();
                self.screen_update_task(screen)
            }
            Message::StepOver => {
                let screen = self.debugger.step_over();
                self.screen_update_task(screen)
            }
            Message::StepFrame => {
                self.frame += 1;
                let screen = self.debugger.step_frame();
                if screen.is_none() {
                    self.running = false;
                }
                self.screen_update_task(screen)
            }
            Message::CaptureFrame => {
                let title = self
                    .debugger
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
            Message::CaptureFrameTo(path) => match self.debugger.capture_frame(&path) {
                Ok(screen) => {
                    self.frame += 1;
                    self.screen_update_task(Some(screen))
                }
                Err(_) => Task::none(),
            },

            Message::SetBreakpoint(address) => {
                self.debugger.set_breakpoint(address);
                Task::none()
            }
            Message::ClearBreakpoint(address) => {
                self.debugger.clear_breakpoint(address);
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
                    self.debugger
                        .set_breakpoint(u16::from_str_radix(&self.breakpoint_input, 16).unwrap());
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
        let colors = M::colors(self.debugger.game_boy(), self.panes.palette());

        let center: Element<'_, app::Message> = if let Some(split_state) = &self.main_split {
            pane_grid(split_state, |_handle, zone, _maximized| {
                let content: Element<'_, app::Message> = match zone {
                    MainSplit::Top => self.panes.view(&self.debugger, &colors),
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
            self.panes.view(&self.debugger, &colors)
        };

        row![
            self.sidebar.view(&self.debugger, &colors),
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
            self.debugger
                .breakpoints()
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

    pub fn subscription(&self) -> Subscription<app::Message> {
        if self.running {
            Subscription::batch([
                time::every(Duration::from_micros(16740)).map(|_| Message::StepFrame.into())
            ])
        } else {
            Subscription::none()
        }
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
        self.debugger.reset();
        self.frame = 0;
    }

    pub fn press_button(&mut self, button: Button) {
        self.debugger.game_boy_mut().press_button(button);
    }

    pub fn release_button(&mut self, button: Button) {
        self.debugger.game_boy_mut().release_button(button);
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
