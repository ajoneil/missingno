use core::fmt;
use std::collections::HashMap;

use iced::{
    Border, Color, Element, Theme,
    widget::{
        container, pane_grid,
        pane_grid::Axis::{Horizontal, Vertical},
        toggler,
    },
};

use crate::app::{
    self,
    ui::{
        fonts, palette,
        icons::Icon,
        sizes::{self as sizes, s, xs},
    },
    debugger::{
        self,
        audio::AudioPane,
        instructions::InstructionsPane,
        ppu::{
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
    panes: Option<pane_grid::State<PaneInstance>>,
    handles: HashMap<DebuggerPane, pane_grid::Pane>,
    palette: PaletteChoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DebuggerPane {
    Screen,
    Instructions,
    Tiles,
    TileMap(TileMapId),
    Sprites,
    Audio,
}

enum PaneInstance {
    Screen(ScreenPane),
    Instructions(InstructionsPane),
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
        if let Some(panes) = &self.panes {
            for (_, pane) in panes.iter() {
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

        Self {
            panes: Some(panes),
            handles,
            palette: PaletteChoice::default(),
        }
    }

    fn construct_pane(pane: DebuggerPane) -> PaneInstance {
        match pane {
            DebuggerPane::Screen => PaneInstance::Screen(ScreenPane::new()),
            DebuggerPane::Instructions => PaneInstance::Instructions(InstructionsPane::new()),
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

                    if let Some(panes) = &mut self.panes {
                        let (last_pane, _) = panes.iter().last().unwrap();
                        let (handle, _) = panes
                            .split(Horizontal, *last_pane, pane_instance)
                            .unwrap();
                        self.handles.insert(pane, handle);
                    } else {
                        let (panes, handle) = pane_grid::State::new(pane_instance);
                        self.handles.insert(pane, handle);
                        self.panes = Some(panes);
                    }
                }
            }
            Message::ClosePane(pane) => {
                if let Some(&handle) = self.handles.get(&pane) {
                    if self.handles.len() == 1 {
                        self.panes = None;
                        self.handles.clear();
                    } else if let Some(panes) = &mut self.panes {
                        panes.close(handle);
                        self.handles.remove(&pane);
                    }
                }
            }

            Message::ResizePane(resize) => {
                if let Some(panes) = &mut self.panes {
                    panes.resize(resize.split, resize.ratio);
                }
            }
            Message::DragPane(drag) => {
                if let pane_grid::DragEvent::Dropped { pane, target } = drag {
                    if let Some(panes) = &mut self.panes {
                        panes.drop(pane, target);
                    }
                }
            }

            Message::Pane(pane_message) => if let Some(panes) = &mut self.panes {
                match &pane_message {
                    PaneMessage::Screen(message) => {
                        panes.iter_mut().for_each(|(_, pane)| {
                            if let PaneInstance::Screen(screen_pane) = pane {
                                screen_pane.update(message.clone());
                            }
                        });
                    }
                    PaneMessage::Sprites(message) => {
                        panes.iter_mut().for_each(|(_, pane)| {
                            if let PaneInstance::Sprites(sprites_pane) = pane {
                                sprites_pane.update(*message);
                            }
                        });
                    }
                }
            },
        }
    }

    pub fn palette(&self) -> &Palette {
        self.palette.palette()
    }

    pub fn set_palette(&mut self, palette: PaletteChoice) {
        self.palette = palette;
        if let Some(panes) = &mut self.panes {
            panes.iter_mut().for_each(|(_, pane)| {
                if let PaneInstance::Screen(screen_pane) = pane {
                    screen_pane.set_palette(palette);
                }
            });
        }
    }

    pub fn view<'a>(
        &'a self,
        debugger: &'a Debugger,
        pal: &'a Palette,
    ) -> Element<'a, app::Message> {
        if let Some(panes) = &self.panes {
            pane_grid(
                panes,
                |_handle, instance, _is_maximized| match instance {
                    PaneInstance::Screen(screen) => screen.content(),
                    PaneInstance::Instructions(instructions) => instructions.content(
                        debugger.game_boy(),
                        debugger.game_boy().cpu().bus_counter,
                        debugger.breakpoints(),
                    ),
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
        } else {
            iced::widget::Space::new()
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .into()
        }
    }

    pub fn plane_shown(&self, plane: DebuggerPane) -> bool {
        self.handles.contains_key(&plane)
    }

    pub fn available_panes(&self) -> &[DebuggerPane] {
        &[
            DebuggerPane::Screen,
            DebuggerPane::Instructions,
            DebuggerPane::Tiles,
            DebuggerPane::TileMap(TileMapId(0)),
            DebuggerPane::TileMap(TileMapId(1)),
            DebuggerPane::Sprites,
            DebuggerPane::Audio,
        ]
    }


}

impl Message {
    pub fn if_shown(pane: DebuggerPane, shown: bool) -> Self {
        if shown {
            Message::ClosePane(pane)
        } else {
            Message::ShowPane(pane)
        }
    }
}

impl DebuggerPane {
    pub fn icon(&self) -> Icon {
        match self {
            DebuggerPane::Screen => Icon::Monitor,
            DebuggerPane::Instructions => Icon::FileText,
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

pub fn title_bar_with_detail<'a>(
    label: &'a str,
    detail: impl Into<Element<'a, app::Message>>,
) -> pane_grid::TitleBar<'a, app::Message> {
    build_title_bar(
        iced::widget::text(label).font(fonts::title()).size(13.0).into(),
        detail,
    )
}

fn build_title_bar<'a>(
    title: Element<'a, app::Message>,
    detail: impl Into<Element<'a, app::Message>>,
) -> pane_grid::TitleBar<'a, app::Message> {
    pane_grid::TitleBar::new(container(title).padding([xs(), s()]))
        // +1px top padding nudge: the detail font (monospace 11px) is shorter
        // than the title font (Chakra Petch 13px), so it needs a small offset
        // to visually center within the title bar height.
        .controls(Element::from(
            container(detail).padding([xs() + 1.0, s()]),
        ))
        .always_show_controls()
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
