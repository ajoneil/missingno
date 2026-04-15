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
    emulator::Emulator,
    screen::{GameBoyScreen, ScreenView, SgbScreen},
    ui::{
        fonts, icons, palette,
        sizes::{s, xs},
    },
};
use missingno_gb::{
    GameBoy,
    joypad::Button,
    ppu::types::palette::{Palette, PaletteChoice},
    sgb::MaskMode,
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

pub struct Debugger {
    debugger: missingno_gb::debugger::Debugger,
    sidebar: Sidebar,
    panes: DebuggerPanes,
    running: bool,
    frame: u64,
    bottom_panes: Option<pane_grid::State<BottomPanel>>,
    bottom_handles: HashMap<BottomPanel, pane_grid::Pane>,
    main_split: Option<pane_grid::State<MainSplit>>,
    breakpoint_input: String,
}

impl Debugger {
    pub fn new(game_boy: GameBoy) -> Self {
        Self {
            debugger: missingno_gb::debugger::Debugger::new(game_boy),
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

    pub fn from_emulator(game_boy: GameBoy, screen_view: ScreenView) -> Self {
        Self {
            debugger: missingno_gb::debugger::Debugger::new(game_boy),
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

    pub fn game_boy(&self) -> &GameBoy {
        self.debugger.game_boy()
    }

    pub fn game_boy_mut(&mut self) -> &mut GameBoy {
        self.debugger.game_boy_mut()
    }

    pub fn disable_debugger(self, use_sgb_colors: bool) -> Emulator {
        let screen_view = self.panes.take_screen_view();
        Emulator::from_debugger(self.debugger.game_boy_take(), screen_view, use_sgb_colors)
    }

    fn screen_update_task(
        &self,
        screen: Option<missingno_gb::ppu::screen::Screen>,
    ) -> Task<app::Message> {
        let video_enabled = self.debugger.game_boy().ppu().control().video_enabled();
        let display = if let Some(sgb) = self.debugger.game_boy().sgb() {
            let render_data = sgb.render_data(video_enabled);
            if sgb.mask_mode == MaskMode::Freeze {
                SgbScreen::Freeze(render_data).into()
            } else if let Some(screen) = screen {
                SgbScreen::Display(screen, render_data).into()
            } else {
                return Task::none();
            }
        } else if !video_enabled {
            GameBoyScreen::Off.into()
        } else if let Some(screen) = screen {
            GameBoyScreen::Display(screen).into()
        } else {
            return Task::none();
        };
        Task::done(screen::Message::Update(display).into())
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
        let pal = self.display_palette();

        let center: Element<'_, app::Message> = if let Some(split_state) = &self.main_split {
            pane_grid(split_state, |_handle, zone, _maximized| {
                let content: Element<'_, app::Message> = match zone {
                    MainSplit::Top => self.panes.view(&self.debugger, pal),
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
            self.panes.view(&self.debugger, pal)
        };

        row![
            self.sidebar.view(&self.debugger, pal),
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

    fn display_palette(&self) -> &Palette {
        if self.debugger.game_boy().sgb().is_some() {
            &Palette::CLASSIC
        } else {
            self.panes.palette()
        }
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
