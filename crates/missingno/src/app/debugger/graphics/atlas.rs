//! The tile atlas pane: a family's decoded tile grid, coloured by the palette
//! the use-site would apply. Frontend-shaded atlases (DMG) resolve through the
//! user palette; core-owned atlases (CGB) preview in neutral greyscale by
//! default, with a picker over the named CRAM palettes, and a selector across
//! the atlases (the CGB VRAM banks).

use std::fmt;

use iced::{
    Element,
    Length::Fill,
    widget::{pick_list, row, scrollable, shader},
};

use crate::app::{
    self,
    console::ConsoleColors,
    debugger::{
        graphics::{ATLAS_COLUMNS, atlas_texture},
        panes::{self, pane, running_placeholder, title_bar, title_bar_with_detail},
    },
    texture_renderer::TextureRenderer,
    ui::fonts,
};
use missingno_core::graphics::{GraphicsView, PaletteSet};
use missingno_gb::ppu::types::palette::{Palette, PaletteIndex};

/// Display scale of the atlas texture.
const SCALE: u32 = 2;

pub struct TileAtlasPane {
    selected_atlas: usize,
    /// 0 previews the core-owned atlas in greyscale; `n` picks the
    /// `n-1`th named palette. Ignored for a frontend-shaded atlas.
    selected_palette: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum Message {
    SelectAtlas(usize),
    SelectPalette(usize),
}

impl From<Message> for app::Message {
    fn from(val: Message) -> Self {
        panes::Message::Pane(panes::PaneMessage::Tiles(val)).into()
    }
}

/// A `pick_list` row carrying the index it selects.
#[derive(Clone, PartialEq)]
struct Choice {
    index: usize,
    label: String,
}

impl fmt::Display for Choice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

impl TileAtlasPane {
    pub fn new() -> Self {
        Self {
            selected_atlas: 0,
            selected_palette: 0,
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::SelectAtlas(index) => {
                self.selected_atlas = index;
                self.selected_palette = 0;
            }
            Message::SelectPalette(index) => self.selected_palette = index,
        }
    }

    fn content<'a>(
        &'a self,
        graphics: &GraphicsView,
        colors: &ConsoleColors,
    ) -> iced::widget::pane_grid::Content<'a, app::Message> {
        let Some(atlas) = graphics.atlases.get(self.selected_atlas) else {
            return running_placeholder("Tiles");
        };

        let (width, height, pixels) = match &atlas.palettes {
            PaletteSet::FrontendShades => {
                let palette = *colors.tiles_palette();
                atlas_texture(atlas, ATLAS_COLUMNS, move |index| {
                    palette.color(PaletteIndex(index))
                })
            }
            PaletteSet::Owned(named) => {
                let chosen = self
                    .selected_palette
                    .checked_sub(1)
                    .and_then(|i| named.get(i));
                atlas_texture(atlas, ATLAS_COLUMNS, |index| match chosen {
                    Some(named) => *named
                        .colors
                        .get(index as usize)
                        .unwrap_or(&rgb::RGB8::new(0, 0, 0)),
                    None => Palette::CLASSIC.color(PaletteIndex(index)),
                })
            }
        };

        let renderer = TextureRenderer::with_pixels(width, height, pixels);
        let body = scrollable(
            shader(renderer)
                .width(iced::Length::Fixed((width * SCALE) as f32))
                .height(iced::Length::Fixed((height * SCALE) as f32)),
        )
        .width(Fill);

        pane(
            self.title_bar(graphics, atlas.palettes.clone()),
            body.into(),
        )
    }

    fn title_bar<'a>(
        &self,
        graphics: &GraphicsView,
        palettes: PaletteSet,
    ) -> iced::widget::pane_grid::TitleBar<'a, app::Message> {
        let atlas_picker = (graphics.atlases.len() > 1).then(|| {
            let choices: Vec<Choice> = graphics
                .atlases
                .iter()
                .enumerate()
                .map(|(index, atlas)| Choice {
                    index,
                    label: atlas.label.clone(),
                })
                .collect();
            let selected = choices.get(self.selected_atlas).cloned();
            picker(choices, selected, |choice| {
                Message::SelectAtlas(choice.index)
            })
        });

        let palette_picker = match palettes {
            PaletteSet::Owned(named) => {
                let mut choices = vec![Choice {
                    index: 0,
                    label: "Greyscale".into(),
                }];
                choices.extend(named.iter().enumerate().map(|(index, named)| Choice {
                    index: index + 1,
                    label: named.label.clone(),
                }));
                let selected = choices.get(self.selected_palette).cloned();
                Some(picker(choices, selected, |choice| {
                    Message::SelectPalette(choice.index)
                }))
            }
            PaletteSet::FrontendShades => None,
        };

        match (atlas_picker, palette_picker) {
            (Some(atlas), Some(palette)) => {
                title_bar_with_detail("Tiles", row![atlas, palette].spacing(6.0))
            }
            (Some(control), None) | (None, Some(control)) => {
                title_bar_with_detail("Tiles", control)
            }
            (None, None) => title_bar("Tiles"),
        }
    }
}

/// A compact `pick_list` styled for a pane title bar.
fn picker(
    choices: Vec<Choice>,
    selected: Option<Choice>,
    on_select: fn(Choice) -> Message,
) -> Element<'static, app::Message> {
    pick_list(choices, selected, move |choice| on_select(choice).into())
        .font(fonts::monospace())
        .text_size(11.0)
        .padding([1.0, 4.0])
        .into()
}

impl panes::Pane for TileAtlasPane {
    fn kind(&self) -> panes::DebuggerPane {
        panes::DebuggerPane::Tiles
    }

    fn view<'a>(
        &'a self,
        ctx: Option<&panes::PaneContext<'_>>,
    ) -> iced::widget::pane_grid::Content<'a, app::Message> {
        match (ctx.and_then(|ctx| ctx.graphics), ctx.and_then(|ctx| ctx.gb)) {
            (Some(graphics), Some(gb)) => self.content(graphics, gb.colors),
            _ => running_placeholder("Tiles"),
        }
    }

    fn on_message(&mut self, message: &panes::PaneMessage) {
        if let panes::PaneMessage::Tiles(message) = message {
            self.update(*message);
        }
    }
}
