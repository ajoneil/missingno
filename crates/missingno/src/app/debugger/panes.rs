use core::fmt;
use std::collections::HashMap;

use iced::{
    Border, Color, Element, Theme,
    widget::{
        button, column, container, pane_grid, tooltip,
        pane_grid::Axis::{Horizontal, Vertical},
        toggler,
    },
};

use crate::app::{
    self,
    ui::{
        fonts, palette,
        icons::{self, Icon},
        sizes::{self as sizes, s, xs},
    },
    debugger::{
        self,
        audio::AudioPane,
        instructions::InstructionsPane,
        ppu::{
            PpuPane,
            sprites::{self, SpritesPane},
            tile_maps::TileMapPane,
            tiles::TilesPane,
        },
        screen::{self, ScreenPane},
    },
    screen::ScreenView,
};
use missingno_gb::debugger::Debugger;
use missingno_gb::ppu::types::{
    palette::{Palette, PaletteChoice},
    tile_maps::TileMapId,
};

#[derive(Debug, Clone)]
pub enum Message {
    ShowPane(DebuggerPane),
    #[allow(dead_code)]
    ClosePane(DebuggerPane),

    ResizePane(pane_grid::ResizeEvent),
    DragPane(pane_grid::DragEvent),

    Pane(PaneMessage),
}

#[derive(Debug, Clone)]
pub enum PaneMessage {
    Screen(screen::Message),
    Sprites(sprites::Message),
}

impl From<Message> for app::Message {
    fn from(message: Message) -> Self {
        app::Message::Debugger(debugger::Message::Pane(message))
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
    Ppu,
    Tiles,
    TileMap(TileMapId),
    Sprites,
    Audio,
}

enum PaneInstance {
    Screen(ScreenPane),
    Instructions(InstructionsPane),
    Ppu(PpuPane),
    Tiles(TilesPane),
    TileMap(TileMapPane),
    Sprites(SpritesPane),
    Audio(AudioPane),
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
                    screen: view.screen.clone(),
                    palette: view.palette,
                    sgb_render_data: view.sgb_render_data,
                    use_sgb_colors: view.use_sgb_colors,
                };
            }
        }
        ScreenView::new()
    }

    fn build(screen_pane: ScreenPane) -> Self {
        let mut handles = HashMap::new();

        let (mut panes, instructions_handle) =
            pane_grid::State::new(Self::construct_pane(DebuggerPane::Instructions));
        handles.insert(DebuggerPane::Instructions, instructions_handle);

        let (screen_handle, split) = panes
            .split(Vertical, instructions_handle, PaneInstance::Screen(screen_pane))
            .unwrap();
        handles.insert(DebuggerPane::Screen, screen_handle);
        panes.resize(split, 1.0 / 3.0);

        let (ppu_handle, split) = panes
            .split(
                Horizontal,
                screen_handle,
                Self::construct_pane(DebuggerPane::Ppu),
            )
            .unwrap();
        handles.insert(DebuggerPane::Ppu, ppu_handle);
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
            DebuggerPane::Ppu => PaneInstance::Ppu(PpuPane::new()),
            DebuggerPane::Tiles => PaneInstance::Tiles(TilesPane::new()),
            DebuggerPane::TileMap(map) => PaneInstance::TileMap(TileMapPane::new(map)),
            DebuggerPane::Sprites => PaneInstance::Sprites(SpritesPane::new()),
            DebuggerPane::Audio => PaneInstance::Audio(AudioPane::new()),
        }
    }

    pub fn update(&mut self, message: Message) {
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
                if let Some(&handle) = self.handles.get(&pane) {
                    if self.panes.close(handle).is_some() {
                        self.handles.remove(&pane);
                    }
                }
            }

            Message::ResizePane(resize) => self.panes.resize(resize.split, resize.ratio),
            Message::DragPane(drag) => match drag {
                pane_grid::DragEvent::Dropped { pane, target } => self.panes.drop(pane, target),
                _ => {}
            },

            Message::Pane(pane_message) => match &pane_message {
                PaneMessage::Screen(message) => {
                    self.panes.iter_mut().for_each(|(_, pane)| {
                        if let PaneInstance::Screen(screen_pane) = pane {
                            screen_pane.update(message.clone());
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
                    debugger.game_boy().cpu().bus_counter,
                    debugger.breakpoints(),
                ),
                PaneInstance::Ppu(ppu_pane) => ppu_pane.content(debugger.game_boy().ppu(), pal),
                PaneInstance::Tiles(tiles) => tiles.content(debugger.game_boy().vram(), pal),
                PaneInstance::TileMap(tile_map) => {
                    tile_map.content(debugger.game_boy().ppu(), debugger.game_boy().vram(), pal)
                }
                PaneInstance::Sprites(sprites) => {
                    sprites.content(debugger.game_boy().ppu(), debugger.game_boy().vram(), pal)
                }
                PaneInstance::Audio(audio) => audio.content(debugger.game_boy().audio()),
            },
        )
        .on_resize(10.0, |resize| Message::ResizePane(resize).into())
        .on_drag(|drag| Message::DragPane(drag).into())
        .spacing(s())
        .into()
    }

    pub fn plane_shown(&self, plane: DebuggerPane) -> bool {
        self.handles.contains_key(&plane)
    }

    pub fn available_panes(&self) -> &[DebuggerPane] {
        &[
            DebuggerPane::Screen,
            DebuggerPane::Instructions,
            DebuggerPane::Ppu,
            DebuggerPane::Tiles,
            DebuggerPane::TileMap(TileMapId(0)),
            DebuggerPane::TileMap(TileMapId(1)),
            DebuggerPane::Sprites,
            DebuggerPane::Audio,
        ]
    }


    pub fn icon_rail(&self) -> Element<'_, app::Message> {
        use crate::app::debugger::sidebar::tooltip_style;

        let buttons = self.available_panes().iter().map(|&pane| {
            let shown = self.plane_shown(pane);
            let color = if shown { palette::PURPLE } else { palette::SURFACE2 };
            let message = if shown {
                Message::ClosePane(pane)
            } else {
                Message::ShowPane(pane)
            };

            let btn: Element<'_, app::Message> = button(
                icons::m_colored(pane.icon(), color),
            )
            .on_press(message.into())
            .style(button::text)
            .into();

            tooltip(btn, container(iced::widget::text(pane.to_string()).font(fonts::monospace()).size(13.0)).padding([2.0, s()]), tooltip::Position::Left)
                .style(tooltip_style)
                .into()
        });

        container(
            column(buttons).spacing(xs()),
        )
        .padding([s(), xs()])
        .into()
    }
}

impl DebuggerPane {
    fn icon(&self) -> Icon {
        match self {
            DebuggerPane::Screen => Icon::Monitor,
            DebuggerPane::Instructions => Icon::Debug,
            DebuggerPane::Ppu => Icon::Brush,
            DebuggerPane::Tiles => Icon::Grid,
            DebuggerPane::TileMap(_) => Icon::Image,
            DebuggerPane::Sprites => Icon::Human,
            DebuggerPane::Audio => Icon::Sliders,
        }
    }
}

impl fmt::Display for DebuggerPane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DebuggerPane::Screen => write!(f, "Screen"),
            DebuggerPane::Instructions => write!(f, "Instructions"),
            DebuggerPane::Ppu => write!(f, "PPU"),
            DebuggerPane::Tiles => write!(f, "Tiles"),
            DebuggerPane::TileMap(map) => write!(f, "{}", map),
            DebuggerPane::Sprites => write!(f, "Sprites"),
            DebuggerPane::Audio => write!(f, "Audio"),
        }
    }
}

pub fn pane<'a>(
    title: pane_grid::TitleBar<'a, app::Message>,
    content: Element<'a, app::Message>,
) -> pane_grid::Content<'a, app::Message> {
    pane_grid::Content::new(container(content).padding([2.0, 2.0]).clip(true))
        .title_bar(title)
        .style(pane_style)
}

pub fn pane_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();

    container::Style {
        background: Some(palette.background.base.color.into()),
        border: Border::default()
            .rounded(sizes::border_s())
            .width(1.0)
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.06)),
        ..Default::default()
    }
}

fn title_style(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(palette::MUTED),
        background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.03).into()),
        ..Default::default()
    }
}

pub fn title_bar(label: &str) -> pane_grid::TitleBar<'_, app::Message> {
    pane_grid::TitleBar::new(
        container(iced::widget::text(label).font(fonts::title()).size(13.0))
            .padding([xs(), s()]),
    )
    .style(title_style)
}

pub fn checkbox_title_bar(
    label: &str,
    checked: bool,
) -> pane_grid::TitleBar<'_, app::Message> {
    pane_grid::TitleBar::new(
        container(toggler(checked).label(label).font(fonts::title()).size(13.0))
            .padding([xs(), s()]),
    )
    .style(title_style)
}
