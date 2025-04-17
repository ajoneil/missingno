use core::fmt;
use std::collections::HashMap;

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
            audio::AudioPane,
            breakpoints::{self, BreakpointsPane},
            cpu::CpuPane,
            instructions::InstructionsPane,
            screen::{self, ScreenPane},
            video::VideoPane,
        },
    },
    debugger::Debugger,
};

#[derive(Debug, Clone)]
pub enum Message {
    ShowPane(DebuggerPane),
    ClosePane(DebuggerPane),

    ResizePane(pane_grid::ResizeEvent),
    DragPane(pane_grid::DragEvent),

    BreakpointsPane(breakpoints::Message),
    ScreenPane(screen::Message),
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        app::Message::Debugger(debugger::Message::Pane(self))
    }
}

pub struct DebuggerPanes {
    panes: pane_grid::State<PaneInstance>,
    handles: HashMap<DebuggerPane, pane_grid::Pane>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DebuggerPane {
    Screen,
    Instructions,
    Breakpoints,
    Cpu,
    Video,
    Audio,
}

enum PaneInstance {
    Screen(ScreenPane),
    Instructions(InstructionsPane),
    Breakpoints(BreakpointsPane),
    Cpu(CpuPane),
    Video(VideoPane),
    Audio(AudioPane),
}

impl DebuggerPanes {
    pub fn new() -> Self {
        let mut handles = HashMap::new();

        let (mut panes, instructions_handle) =
            pane_grid::State::new(Self::construct_pane(DebuggerPane::Instructions));
        handles.insert(DebuggerPane::Instructions, instructions_handle);

        let (screen_handle, split) = panes
            .split(
                Vertical,
                instructions_handle,
                Self::construct_pane(DebuggerPane::Screen),
            )
            .unwrap();
        handles.insert(DebuggerPane::Screen, screen_handle);
        panes.resize(split, 1.0 / 4.0);

        let (video_handle, split) = panes
            .split(
                Vertical,
                screen_handle,
                Self::construct_pane(DebuggerPane::Video),
            )
            .unwrap();
        handles.insert(DebuggerPane::Video, video_handle);
        panes.resize(split, 1.0 / 3.0);

        let (cpu_handle, split) = panes
            .split(
                Horizontal,
                screen_handle,
                Self::construct_pane(DebuggerPane::Cpu),
            )
            .unwrap();
        panes.resize(split, 3.0 / 4.0);
        handles.insert(DebuggerPane::Cpu, cpu_handle);

        let (breakpoints_handle, split) = panes
            .split(
                Horizontal,
                instructions_handle,
                Self::construct_pane(DebuggerPane::Breakpoints),
            )
            .unwrap();
        handles.insert(DebuggerPane::Breakpoints, breakpoints_handle);
        panes.resize(split, 3.0 / 4.0);

        Self { panes, handles }
    }

    fn construct_pane(pane: DebuggerPane) -> PaneInstance {
        match pane {
            DebuggerPane::Screen => PaneInstance::Screen(ScreenPane::new()),
            DebuggerPane::Instructions => PaneInstance::Instructions(InstructionsPane::new()),
            DebuggerPane::Breakpoints => PaneInstance::Breakpoints(BreakpointsPane::new()),
            DebuggerPane::Cpu => PaneInstance::Cpu(CpuPane::new()),
            DebuggerPane::Video => PaneInstance::Video(VideoPane::new()),
            DebuggerPane::Audio => PaneInstance::Audio(AudioPane::new()),
        }
    }

    pub fn update(&mut self, message: Message, debugger: &mut Debugger) {
        match message {
            Message::ShowPane(pane) => {
                if self.handles.get(&pane).is_none() {
                    let pane_instance = Self::construct_pane(pane);

                    if self.panes.is_empty() {
                        let (panes, handle) = pane_grid::State::new(pane_instance);
                        self.handles.insert(pane, handle);
                        self.panes = panes;
                    } else {
                        let (last_pane, _) = self.panes.iter().last().unwrap();
                        let (handle, _) = self
                            .panes
                            .split(Horizontal, *last_pane, pane_instance)
                            .unwrap();
                        self.handles.insert(pane, handle);
                    }
                }
            }
            Message::ClosePane(pane) => {
                if let Some(handle) = self.handles.remove(&pane) {
                    self.panes.close(handle);
                }
            }

            Message::ResizePane(resize) => self.panes.resize(resize.split, resize.ratio),
            Message::DragPane(drag) => match drag {
                pane_grid::DragEvent::Dropped { pane, target } => self.panes.drop(pane, target),
                _ => {}
            },

            Message::BreakpointsPane(message) => {
                if let Some(breakpoints_handle) = self.handles.get(&DebuggerPane::Breakpoints) {
                    match self.panes.get_mut(*breakpoints_handle) {
                        Some(PaneInstance::Breakpoints(breakpoints_pane)) => {
                            breakpoints_pane.update(message, debugger);
                        }
                        _ => {}
                    }
                }
            }

            Message::ScreenPane(message) => {
                if let Some(handle) = self.handles.get(&DebuggerPane::Screen) {
                    match self.panes.get_mut(*handle) {
                        Some(PaneInstance::Screen(screen_pane)) => {
                            screen_pane.update(message);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    pub fn view(&self, debugger: &Debugger) -> Element<'_, app::Message> {
        pane_grid(
            &self.panes,
            |_handle, instance, _is_maximized| match instance {
                PaneInstance::Screen(screen) => screen.content(),
                PaneInstance::Instructions(instructions) => instructions.content(
                    debugger.game_boy().memory_mapped(),
                    debugger.game_boy().cpu().program_counter,
                    debugger.breakpoints(),
                ),
                PaneInstance::Breakpoints(breakpoints) => breakpoints.content(debugger),
                PaneInstance::Cpu(cpu) => cpu.content(debugger),
                PaneInstance::Video(video) => video.content(debugger.game_boy().video()),
                PaneInstance::Audio(audio) => audio.content(debugger.game_boy().audio()),
            },
        )
        .on_resize(10.0, |resize| Message::ResizePane(resize).into())
        .on_drag(|drag| Message::DragPane(drag).into())
        .spacing(m())
        .into()
    }

    pub fn plane_shown(&self, plane: DebuggerPane) -> bool {
        self.handles.contains_key(&plane)
    }

    pub fn available_panes(&self) -> &[DebuggerPane] {
        &[
            DebuggerPane::Screen,
            DebuggerPane::Instructions,
            DebuggerPane::Breakpoints,
            DebuggerPane::Cpu,
            DebuggerPane::Video,
            DebuggerPane::Audio,
        ]
    }

    pub fn unshown_panes(&self) -> Vec<DebuggerPane> {
        self.available_panes()
            .into_iter()
            .filter(|&pane| !self.plane_shown(*pane))
            .cloned()
            .collect()
    }
}

impl fmt::Display for DebuggerPane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DebuggerPane::Screen => write!(f, "Screen"),
            DebuggerPane::Instructions => write!(f, "Instructions"),
            DebuggerPane::Breakpoints => write!(f, "Breakpoints"),
            DebuggerPane::Cpu => write!(f, "CPU"),
            DebuggerPane::Video => write!(f, "Video"),
            DebuggerPane::Audio => write!(f, "Audio"),
        }
    }
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
    closable: Option<DebuggerPane>,
) -> pane_grid::TitleBar<'_, app::Message> {
    tbar(text::m(label).font(fonts::title()).into(), closable)
}

pub fn checkbox_title_bar(
    label: &str,
    checked: bool,
    closable: Option<DebuggerPane>,
) -> pane_grid::TitleBar<'_, app::Message> {
    tbar(
        checkbox(label, checked).font(fonts::title()).into(),
        closable,
    )
}

fn tbar(
    content: Element<'_, app::Message>,
    close_pane: Option<DebuggerPane>,
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
