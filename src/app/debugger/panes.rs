use core::fmt;
use std::collections::HashMap;

use iced::{
    Border, Color, Element, Length, Theme,
    widget::{
        container, pane_grid,
        pane_grid::Axis::{Horizontal, Vertical},
        svg, toggler,
    },
};

use crate::app::{
    self,
    core::{
        buttons, fonts,
        icons::{self, Icon},
        sizes::{m, s},
        text,
    },
    debugger::{
        self,
        audio::AudioPane,
        breakpoints::{self, BreakpointsPane},
        cpu::CpuPane,
        instructions::InstructionsPane,
        playback::PlaybackPane,
        screen::{self, ScreenPane},
        video::{
            VideoPane,
            sprites::{self, SpritesPane},
            tile_maps::TileMapPane,
            tiles::TilesPane,
        },
    },
    screen::ScreenView,
};
use missingno_core::debugger::Debugger;
use missingno_core::game_boy::video::{
    palette::{Palette, PaletteChoice},
    tile_maps::TileMapId,
};

#[derive(Debug, Clone)]
pub enum Message {
    ShowPane(DebuggerPane),
    ClosePane(DebuggerPane),

    ResizePane(pane_grid::ResizeEvent),
    DragPane(pane_grid::DragEvent),

    Pane(PaneMessage),
}

#[derive(Debug, Clone)]
pub enum PaneMessage {
    Breakpoints(breakpoints::Message),
    Screen(screen::Message),
    Sprites(sprites::Message),
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        app::Message::Debugger(debugger::Message::Pane(self))
    }
}

pub struct DebuggerPanes {
    panes: pane_grid::State<PaneInstance>,
    handles: HashMap<DebuggerPane, pane_grid::Pane>,
    palette: PaletteChoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DebuggerPane {
    Screen,
    Instructions,
    Breakpoints,
    Cpu,
    Video,
    Tiles,
    TileMap(TileMapId),
    Sprites,
    Audio,
    Playback,
}

enum PaneInstance {
    Screen(ScreenPane),
    Instructions(InstructionsPane),
    Breakpoints(BreakpointsPane),
    Cpu(CpuPane),
    Video(VideoPane),
    Tiles(TilesPane),
    TileMap(TileMapPane),
    Sprites(SpritesPane),
    Audio(AudioPane),
    Playback(PlaybackPane),
}

impl DebuggerPanes {
    pub fn new() -> Self {
        Self::build(ScreenPane::new())
    }

    pub fn with_screen(screen_view: ScreenView) -> Self {
        Self::build(ScreenPane::with_screen(screen_view))
    }

    pub fn take_screen_view(self) -> ScreenView {
        for (_, pane) in self.panes.iter() {
            if let PaneInstance::Screen(screen_pane) = pane {
                let view = screen_pane.screen_view();
                return ScreenView {
                    screen: view.screen,
                    palette: view.palette,
                    sgb_render_data: view.sgb_render_data,
                };
            }
        }
        ScreenView::new()
    }

    fn build(screen_pane: ScreenPane) -> Self {
        let mut handles = HashMap::new();

        let (mut panes, cpu_handle) =
            pane_grid::State::new(Self::construct_pane(DebuggerPane::Cpu));
        handles.insert(DebuggerPane::Cpu, cpu_handle);

        let (screen_handle, split) = panes
            .split(Vertical, cpu_handle, PaneInstance::Screen(screen_pane))
            .unwrap();
        handles.insert(DebuggerPane::Screen, screen_handle);
        panes.resize(split, 1.0 / 4.0);

        let (video_handle, split) = panes
            .split(
                Horizontal,
                screen_handle,
                Self::construct_pane(DebuggerPane::Video),
            )
            .unwrap();
        handles.insert(DebuggerPane::Video, video_handle);
        panes.resize(split, 3.0 / 4.0);

        let (instructions_handle, split) = panes
            .split(
                Horizontal,
                cpu_handle,
                Self::construct_pane(DebuggerPane::Instructions),
            )
            .unwrap();
        panes.resize(split, 1.0 / 4.0);
        handles.insert(DebuggerPane::Instructions, instructions_handle);

        let (breakpoints_handle, split) = panes
            .split(
                Horizontal,
                instructions_handle,
                Self::construct_pane(DebuggerPane::Breakpoints),
            )
            .unwrap();
        handles.insert(DebuggerPane::Breakpoints, breakpoints_handle);
        panes.resize(split, 3.0 / 4.0);

        Self {
            panes,
            handles,
            palette: PaletteChoice::default(),
        }
    }

    fn construct_pane(pane: DebuggerPane) -> PaneInstance {
        match pane {
            DebuggerPane::Screen => PaneInstance::Screen(ScreenPane::new()),
            DebuggerPane::Instructions => PaneInstance::Instructions(InstructionsPane::new()),
            DebuggerPane::Breakpoints => PaneInstance::Breakpoints(BreakpointsPane::new()),
            DebuggerPane::Cpu => PaneInstance::Cpu(CpuPane::new()),
            DebuggerPane::Video => PaneInstance::Video(VideoPane::new()),
            DebuggerPane::Tiles => PaneInstance::Tiles(TilesPane::new()),
            DebuggerPane::TileMap(map) => PaneInstance::TileMap(TileMapPane::new(map)),
            DebuggerPane::Sprites => PaneInstance::Sprites(SpritesPane::new()),
            DebuggerPane::Audio => PaneInstance::Audio(AudioPane::new()),
            DebuggerPane::Playback => PaneInstance::Playback(PlaybackPane::new()),
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

            Message::Pane(pane_message) => match &pane_message {
                PaneMessage::Breakpoints(message) => {
                    self.panes.iter_mut().for_each(|(_, pane)| {
                        if let PaneInstance::Breakpoints(breakpoints_pane) = pane {
                            breakpoints_pane.update(message, debugger);
                        }
                    });
                }
                PaneMessage::Screen(message) => {
                    self.panes.iter_mut().for_each(|(_, pane)| {
                        if let PaneInstance::Screen(screen_pane) = pane {
                            screen_pane.update(*message);
                        }
                    });
                }
                PaneMessage::Sprites(message) => {
                    self.panes.iter_mut().for_each(|(_, pane)| {
                        if let PaneInstance::Sprites(sprites_pane) = pane {
                            sprites_pane.update(*message);
                        }
                    });
                }
            },
        }
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.palette = palette;
        self.panes.iter_mut().for_each(|(_, pane)| {
            if let PaneInstance::Screen(screen_pane) = pane {
                screen_pane.set_palette(palette);
            }
        });
    }

    pub fn view<'a>(
        &'a self,
        debugger: &'a Debugger,
        app_debugger: &'a super::Debugger,
    ) -> Element<'a, app::Message> {
        let pal = if debugger.game_boy().sgb().is_some() {
            &Palette::CLASSIC
        } else {
            self.palette.palette()
        };
        pane_grid(
            &self.panes,
            |_handle, instance, _is_maximized| match instance {
                PaneInstance::Screen(screen) => screen.content(),
                PaneInstance::Instructions(instructions) => instructions.content(
                    debugger.game_boy(),
                    debugger.game_boy().cpu().program_counter,
                    debugger.breakpoints(),
                ),
                PaneInstance::Breakpoints(breakpoints) => breakpoints.content(debugger),
                PaneInstance::Cpu(cpu) => cpu.content(debugger),
                PaneInstance::Video(video) => video.content(debugger.game_boy().video(), pal),
                PaneInstance::Tiles(tiles) => tiles.content(debugger.game_boy().vram(), pal),
                PaneInstance::TileMap(tile_map) => {
                    tile_map.content(debugger.game_boy().video(), debugger.game_boy().vram(), pal)
                }
                PaneInstance::Sprites(sprites) => {
                    sprites.content(debugger.game_boy().video(), debugger.game_boy().vram(), pal)
                }
                PaneInstance::Audio(audio) => audio.content(debugger.game_boy().audio()),
                PaneInstance::Playback(playback) => playback.content(app_debugger),
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
            DebuggerPane::Tiles,
            DebuggerPane::TileMap(TileMapId(0)),
            DebuggerPane::TileMap(TileMapId(1)),
            DebuggerPane::Sprites,
            DebuggerPane::Audio,
            DebuggerPane::Playback,
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
            DebuggerPane::Tiles => write!(f, "Tiles"),
            DebuggerPane::TileMap(map) => write!(f, "{}", map),
            DebuggerPane::Sprites => write!(f, "Sprites"),
            DebuggerPane::Audio => write!(f, "Audio"),
            DebuggerPane::Playback => write!(f, "Playback"),
        }
    }
}

pub fn pane<'a>(
    title: pane_grid::TitleBar<'a, app::Message>,
    content: Element<'a, app::Message>,
) -> pane_grid::Content<'a, app::Message> {
    pane_grid::Content::new(container(content).padding([2.0, 2.0]))
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

pub fn title_bar(label: &str, pane: DebuggerPane) -> pane_grid::TitleBar<'_, app::Message> {
    tbar(text::m(label).font(fonts::title()).into(), pane)
}

pub fn checkbox_title_bar(
    label: &str,
    checked: bool,
    pane: DebuggerPane,
) -> pane_grid::TitleBar<'_, app::Message> {
    tbar(
        toggler(checked).label(label).font(fonts::title()).into(),
        pane,
    )
}

fn tbar(
    content: Element<'_, app::Message>,
    pane: DebuggerPane,
) -> pane_grid::TitleBar<'_, app::Message> {
    pane_grid::TitleBar::new(container(content).padding(s()))
        .style(title_style)
        .controls(pane_grid::Controls::new(
            container(
                buttons::standard(icons::m(Icon::Close).style(|_, _| svg::Style {
                    color: Some(Color::BLACK),
                }))
                .on_press(Message::ClosePane(pane).into()),
            )
            .center_y(Length::Fixed(m() + 2.0 * s())),
        ))
}
