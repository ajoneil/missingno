//! The tile atlas pane: a family's decoded tile grid, coloured by the palette
//! the use-site would apply. Frontend-shaded atlases (DMG) resolve through the
//! user palette; core-owned atlases (CGB) preview in neutral greyscale by
//! default, with a picker over the named CRAM palettes, and a selector across
//! the atlases (the CGB VRAM banks).

use std::fmt;

use iced::{
    Element,
    Length::Fill,
    widget::{column, pick_list, row, rule, scrollable, shader, text, tooltip},
};
use rgb::RGB8;

use crate::app::{
    self,
    console::ConsoleColors,
    debugger::{
        graphics::{ATLAS_COLUMNS, atlas_span_texture, atlas_texture},
        panes::{self, pane, running_placeholder, title_bar, title_bar_with_detail},
        sidebar::help_tooltip,
    },
    ui::{fonts, palette, sizes::s},
};
use missingno_core::graphics::{AtlasRegion, GraphicsView, PaletteSet, TileAtlas};
use missingno_gb::ppu::types::palette::{Palette, PaletteIndex};
use missingno_iced::TextureRenderer;

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
        close: iced::widget::pane_grid::Pane,
    ) -> iced::widget::pane_grid::Content<'a, app::Message> {
        if graphics.atlases.is_empty() {
            return running_placeholder("Tiles", close);
        }
        // Clamp a stale stored selection to a live atlas so the pane always
        // renders with its picker, rather than stranding on a bare placeholder.
        let atlas = &graphics.atlases[self.displayed_atlas(graphics)];

        let resolve = self.tile_resolver(atlas, colors);
        let body = region_layout(atlas, resolve.as_ref());

        pane(
            self.title_bar(graphics, atlas.palettes.clone(), close),
            body,
        )
    }

    /// The atlas index actually shown: the stored selection clamped to a live
    /// atlas. Callers must have checked `graphics.atlases` is non-empty.
    fn displayed_atlas(&self, graphics: &GraphicsView) -> usize {
        self.selected_atlas.min(graphics.atlases.len() - 1)
    }

    /// The palette index → colour map for `atlas`: the user's DMG shades for a
    /// frontend-shaded atlas, or the picked CRAM palette (greyscale when none)
    /// for a core-owned one.
    fn tile_resolver(&self, atlas: &TileAtlas, colors: &ConsoleColors) -> Box<dyn Fn(u8) -> RGB8> {
        match &atlas.palettes {
            PaletteSet::FrontendShades => {
                let palette = *colors.tiles_palette();
                Box::new(move |index| palette.color(PaletteIndex(index)))
            }
            PaletteSet::Owned(named) => {
                let chosen = self
                    .selected_palette
                    .checked_sub(1)
                    .and_then(|i| named.get(i))
                    .cloned();
                Box::new(move |index| match &chosen {
                    Some(named) => named
                        .colors
                        .get(index as usize)
                        .copied()
                        .unwrap_or(RGB8::new(0, 0, 0)),
                    None => Palette::CLASSIC.color(PaletteIndex(index)),
                })
            }
        }
    }

    fn title_bar<'a>(
        &self,
        graphics: &GraphicsView,
        palettes: PaletteSet,
        close: iced::widget::pane_grid::Pane,
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
            let selected = choices.get(self.displayed_atlas(graphics)).cloned();
            picker(choices, selected, close, |choice| {
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
                Some(picker(choices, selected, close, |choice| {
                    Message::SelectPalette(choice.index)
                }))
            }
            PaletteSet::FrontendShades => None,
        };

        match (atlas_picker, palette_picker) {
            (Some(atlas), Some(palette)) => {
                title_bar_with_detail("Tiles", row![atlas, palette].spacing(6.0), close)
            }
            (Some(control), None) | (None, Some(control)) => {
                title_bar_with_detail("Tiles", control, close)
            }
            (None, None) => title_bar("Tiles", close),
        }
    }
}

/// The atlas body laid out by region: each region's tiles under a muted header
/// separating it from the last (the sidebar Rule idiom, scaled to the pane). An
/// unannotated atlas (no regions) draws as one 16-wide grid.
fn region_layout<'a>(atlas: &TileAtlas, resolve: &dyn Fn(u8) -> RGB8) -> Element<'a, app::Message> {
    if atlas.regions.is_empty() {
        let (width, height, pixels) = atlas_texture(atlas, ATLAS_COLUMNS, resolve);
        return scrollable(region_texture(width, height, pixels))
            .width(Fill)
            .into();
    }

    let mut layout = column![].spacing(s());
    for region in &atlas.regions {
        let (width, height, pixels) =
            atlas_span_texture(atlas, region.start, region.len, ATLAS_COLUMNS, resolve);
        layout = layout
            .push(region_header(region))
            .push(region_texture(width, height, pixels));
    }
    scrollable(layout).width(Fill).into()
}

/// One region's tile grid as a scaled shader texture.
fn region_texture<'a>(width: u32, height: u32, pixels: Vec<u8>) -> Element<'a, app::Message> {
    let renderer = TextureRenderer::with_pixels(width, height, pixels);
    shader(renderer)
        .width(iced::Length::Fixed((width * SCALE) as f32))
        .height(iced::Length::Fixed((height * SCALE) as f32))
        .into()
}

/// A region's muted label above a faint rule, its address-range help (if any) on
/// hover.
fn region_header(region: &AtlasRegion) -> Element<'static, app::Message> {
    let label = text(region.label)
        .font(fonts::monospace())
        .size(11.0)
        .color(palette::MUTED);
    let header: Element<'static, app::Message> = match region.help {
        Some(help) => help_tooltip(label, help, 11.0, tooltip::Position::Right),
        None => label.into(),
    };
    column![rule::horizontal(1), header].spacing(2.0).into()
}

/// A compact `pick_list` styled for a pane title bar, its selection targeted at
/// this instance's handle so sibling tile panes stay put.
fn picker(
    choices: Vec<Choice>,
    selected: Option<Choice>,
    close: iced::widget::pane_grid::Pane,
    on_select: fn(Choice) -> Message,
) -> Element<'static, app::Message> {
    pick_list(choices, selected, move |choice| {
        panes::PaneMessage::Tiles(on_select(choice)).to(close)
    })
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
        id: iced::widget::pane_grid::Pane,
    ) -> iced::widget::pane_grid::Content<'a, app::Message> {
        match (
            ctx.and_then(|ctx| ctx.graphics),
            ctx.and_then(|ctx| ctx.colors),
        ) {
            (Some(graphics), Some(colors)) => self.content(graphics, colors, id),
            _ => running_placeholder("Tiles", id),
        }
    }

    fn on_message(&mut self, message: &panes::PaneMessage) {
        if let panes::PaneMessage::Tiles(message) = message {
            self.update(*message);
        }
    }

    fn source_index(&self) -> Option<usize> {
        Some(self.selected_atlas)
    }

    fn set_source_index(&mut self, index: usize) {
        self.selected_atlas = index;
        self.selected_palette = 0;
    }
}
