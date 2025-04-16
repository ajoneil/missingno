use core::fmt;

use iced::{
    Alignment::Center,
    Border, Color, Element,
    Length::Fill,
    Theme,
    widget::{
        checkbox, container, pane_grid,
        pane_grid::Axis::{Horizontal, Vertical},
        row,
    },
};

use crate::{
    app::{
        self,
        core::{
            buttons, fonts,
            sizes::{m, s},
            text,
        },
        debugger::{
            self,
            audio::audio_pane,
            breakpoints::{self, breakpoints_pane},
            cpu::cpu_pane,
            instructions::instructions_pane,
            video::video_pane,
        },
    },
    debugger::Debugger,
};

#[derive(Debug, Clone)]
pub enum Message {
    ShowPane(AvailablePanes),
    ClosePane(AvailablePanes),

    ResizePane(pane_grid::ResizeEvent),
    DragPane(pane_grid::DragEvent),

    Breakpoint(breakpoints::Message),
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        app::Message::Debugger(debugger::Message::Pane(self))
    }
}

pub struct Panes {
    state: pane_grid::State<PaneState>,
    instructions: Option<pane_grid::Pane>,
    breakpoints: Option<pane_grid::Pane>,
    cpu: Option<pane_grid::Pane>,
    video: Option<pane_grid::Pane>,
    audio: Option<pane_grid::Pane>,
}

impl Panes {
    pub fn new() -> Self {
        let (mut state, instructions) = pane_grid::State::new(PaneState::Instructions);
        let (cpu, split) = state.split(Vertical, instructions, PaneState::Cpu).unwrap();
        state.resize(split, 1.0 / 4.0);

        let (breakpoint, split) = state
            .split(
                Horizontal,
                instructions,
                PaneState::Breakpoints(breakpoints::State::new()),
            )
            .unwrap();
        state.resize(split, 3.0 / 4.0);

        let (video, split) = state.split(Vertical, cpu, PaneState::Video).unwrap();
        state.resize(split, 1.0 / 3.0);

        Self {
            state,
            instructions: Some(instructions),
            breakpoints: Some(breakpoint),
            cpu: Some(cpu),
            video: Some(video),
            audio: None,
        }
    }

    pub fn update(&mut self, message: Message, debugger: &mut Debugger) {
        match message {
            Message::Breakpoint(message) => {
                if let Some(PaneState::Breakpoints(breakpoints_state)) =
                    self.state.get_mut(self.breakpoints.unwrap())
                {
                    breakpoints_state.update(message, debugger);
                }
            }

            Message::ShowPane(pane) => {
                if let Some((last_pane, _)) = self.state.iter().last() {
                    match pane {
                        AvailablePanes::Instructions => {
                            let (instructions, _) = self
                                .state
                                .split(Horizontal, *last_pane, PaneState::Instructions)
                                .unwrap();
                            self.instructions = Some(instructions);
                        }
                        AvailablePanes::Breakpoints => {
                            let (breakpoints, _) = self
                                .state
                                .split(
                                    Horizontal,
                                    *last_pane,
                                    PaneState::Breakpoints(breakpoints::State::new()),
                                )
                                .unwrap();
                            self.breakpoints = Some(breakpoints);
                        }
                        AvailablePanes::Cpu => {
                            let (cpu, _) = self
                                .state
                                .split(Horizontal, *last_pane, PaneState::Cpu)
                                .unwrap();
                            self.cpu = Some(cpu)
                        }
                        AvailablePanes::Video => {
                            let (video, _) = self
                                .state
                                .split(Horizontal, *last_pane, PaneState::Video)
                                .unwrap();
                            self.video = Some(video)
                        }
                        AvailablePanes::Audio => {
                            let (audio, _) = self
                                .state
                                .split(Horizontal, *last_pane, PaneState::Audio)
                                .unwrap();
                            self.audio = Some(audio)
                        }
                    };
                }
            }

            Message::ClosePane(pane) => {
                if self.state.len() > 1 {
                    match pane {
                        AvailablePanes::Instructions => {
                            self.state.close(self.instructions.unwrap());
                            self.instructions = None;
                        }
                        AvailablePanes::Breakpoints => {
                            self.state.close(self.breakpoints.unwrap());
                            self.breakpoints = None;
                        }
                        AvailablePanes::Cpu => {
                            self.state.close(self.cpu.unwrap());
                            self.cpu = None;
                        }
                        AvailablePanes::Video => {
                            self.state.close(self.video.unwrap());
                            self.video = None;
                        }
                        AvailablePanes::Audio => {
                            self.state.close(self.audio.unwrap());
                            self.audio = None;
                        }
                    }
                }
            }

            Message::ResizePane(resize) => self.state.resize(resize.split, resize.ratio),
            Message::DragPane(drag) => match drag {
                pane_grid::DragEvent::Dropped { pane, target } => self.state.drop(pane, target),
                _ => {}
            },
        }
    }

    pub fn view<'a>(&'a self, debugger: &'a Debugger) -> Element<'a, app::Message> {
        pane_grid(
            &self.state,
            |_pane, pane_state, _is_maximized| match pane_state {
                PaneState::Instructions => instructions_pane(
                    debugger.game_boy().memory_mapped(),
                    debugger.game_boy().cpu().program_counter,
                    debugger.breakpoints(),
                ),
                PaneState::Breakpoints(breakpoint_state) => {
                    breakpoints_pane(debugger, breakpoint_state)
                }
                PaneState::Cpu => cpu_pane(debugger),
                PaneState::Video => video_pane(debugger.game_boy().video()),
                PaneState::Audio => audio_pane(debugger.game_boy().audio()),
            },
        )
        .on_resize(10.0, |resize| Message::ResizePane(resize).into())
        .on_drag(|drag| Message::DragPane(drag).into())
        .spacing(m())
        .into()
    }

    pub fn plane_shown(&self, plane: AvailablePanes) -> bool {
        match plane {
            AvailablePanes::Instructions => self.instructions.is_some(),
            AvailablePanes::Breakpoints => self.breakpoints.is_some(),
            AvailablePanes::Cpu => self.cpu.is_some(),
            AvailablePanes::Video => self.video.is_some(),
            AvailablePanes::Audio => self.audio.is_some(),
        }
    }

    pub fn available_panes(&self) -> &[AvailablePanes] {
        &[
            AvailablePanes::Instructions,
            AvailablePanes::Breakpoints,
            AvailablePanes::Cpu,
            AvailablePanes::Video,
            AvailablePanes::Audio,
        ]
    }

    pub fn unshown_panes(&self) -> Vec<AvailablePanes> {
        self.available_panes()
            .into_iter()
            .filter(|&pane| !self.plane_shown(*pane))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvailablePanes {
    Instructions,
    Breakpoints,
    Cpu,
    Video,
    Audio,
}

impl fmt::Display for AvailablePanes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AvailablePanes::Instructions => write!(f, "Instructions"),
            AvailablePanes::Breakpoints => write!(f, "Breakpoints"),
            AvailablePanes::Cpu => write!(f, "CPU"),
            AvailablePanes::Video => write!(f, "Video"),
            AvailablePanes::Audio => write!(f, "Audio"),
        }
    }
}

pub enum PaneState {
    Instructions,
    Breakpoints(breakpoints::State),
    Cpu,
    Video,
    Audio,
}

pub fn pane<'a>(
    title: pane_grid::TitleBar<'a, app::Message>,
    content: Element<'a, app::Message>,
) -> pane_grid::Content<'a, app::Message> {
    pane_grid::Content::new(container(content).padding(m()))
        .title_bar(title)
        .style(pane_style)
}

pub fn pane_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        border: Border {
            width: 2.0,
            color: palette.primary.strong.color,
            ..Border::default()
        },
        background: Some(palette.background.base.color.into()),
        ..Default::default()
    }
}

pub fn title_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        text_color: Some(palette.primary.strong.text),
        background: Some(palette.primary.strong.color.into()),
        ..Default::default()
    }
}

pub fn title_bar(
    label: &str,
    closable: Option<AvailablePanes>,
) -> pane_grid::TitleBar<'_, app::Message> {
    tbar(text::m(label).font(fonts::title()).into(), closable)
}

pub fn checkbox_title_bar(
    label: &str,
    checked: bool,
    closable: Option<AvailablePanes>,
) -> pane_grid::TitleBar<'_, app::Message> {
    tbar(
        checkbox(label, checked).font(fonts::title()).into(),
        closable,
    )
}

fn tbar(
    content: Element<'_, app::Message>,
    close_pane: Option<AvailablePanes>,
) -> pane_grid::TitleBar<'_, app::Message> {
    pane_grid::TitleBar::new(if let Some(pane) = close_pane {
        row![
            content,
            container(
                buttons::text(text::m("x").font(fonts::title()).color(Color::BLACK))
                    .on_press(Message::ClosePane(pane).into())
            )
            .align_right(Fill)
        ]
        .align_y(Center)
        .into()
    } else {
        content
    })
    .style(title_style)
    .padding(s())
}
