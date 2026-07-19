//! The tile map pane: a background/name-table map composited from its atlas,
//! with the on-screen viewports (the Game Boy background and window rectangles)
//! drawn over it. A map dropdown in the title bar selects which map this
//! instance shows; several instances can watch different maps at once.

use iced::{
    Length,
    Length::Fill,
    Rectangle, Renderer, Theme, mouse,
    widget::{
        Stack,
        canvas::{Frame, Geometry, Path, Program, Stroke},
        container, pane_grid, pick_list, responsive, shader,
    },
};

use crate::app::{
    self,
    console::ConsoleColors,
    debugger::{
        graphics::{flipped, wrapping_parts},
        panes::{self, pane, running_placeholder, title_bar, title_bar_with_detail},
        sidebar::tone_color,
    },
    texture_renderer::TextureRenderer,
    ui::fonts,
};
use missingno_core::graphics::{GraphicsView, TileMap, Viewport};
use missingno_gb::ppu::types::palette::PaletteIndex;
use missingno_gb::ppu::types::tiles::TileMapId;

pub struct TileMapPane {
    tile_map: TileMapId,
}

#[derive(Debug, Clone, Copy)]
pub enum Message {
    SelectMap(TileMapId),
}

impl TileMapPane {
    pub fn new(tile_map: TileMapId) -> Self {
        Self { tile_map }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::SelectMap(id) => self.tile_map = id,
        }
    }

    fn content<'a>(
        &'a self,
        graphics: &GraphicsView,
        colors: &ConsoleColors,
        close: pane_grid::Pane,
    ) -> pane_grid::Content<'a, app::Message> {
        let Some(map) = graphics.maps.get(self.tile_map.0 as usize) else {
            return running_placeholder("Tile Map", close);
        };

        let (width, height, pixels) = compose(map, graphics, colors);
        let map_size = width.max(height) as f32;
        let pixels: std::sync::Arc<[u8]> = pixels.into();
        let overlay = ViewportOverlay {
            viewports: map.viewports.clone(),
            map_size,
        };

        pane(
            self.title_bar(graphics, close),
            responsive(move |size| {
                let fit = size.width.min(size.height);
                let renderer = TextureRenderer::with_pixels(width, height, pixels.clone());
                container(
                    Stack::new()
                        .push(
                            shader(renderer)
                                .width(Length::Fixed(fit))
                                .height(Length::Fixed(fit)),
                        )
                        .push(
                            iced::widget::canvas(overlay.clone())
                                .width(Length::Fixed(fit))
                                .height(Length::Fixed(fit)),
                        ),
                )
                .center(Fill)
                .into()
            })
            .into(),
        )
    }

    /// The title bar with a map picker, when the family exposes more than one
    /// map; a plain title otherwise. The picker targets this instance's handle.
    fn title_bar<'a>(
        &self,
        graphics: &GraphicsView,
        close: pane_grid::Pane,
    ) -> pane_grid::TitleBar<'a, app::Message> {
        if graphics.maps.len() <= 1 {
            return title_bar("Tile Map", close);
        }
        let choices: Vec<MapChoice> = (0..graphics.maps.len())
            .map(|index| MapChoice(TileMapId(index as u8)))
            .collect();
        let picker = pick_list(choices, Some(MapChoice(self.tile_map)), move |choice| {
            panes::PaneMessage::TileMap(Message::SelectMap(choice.0)).to(close)
        })
        .font(fonts::monospace())
        .text_size(11.0)
        .padding([1.0, 4.0]);
        title_bar_with_detail("Tile Map", picker, close)
    }
}

/// A map-picker row naming one map.
#[derive(Clone, Copy, PartialEq)]
struct MapChoice(TileMapId);

impl std::fmt::Display for MapChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Map {}", self.0.0)
    }
}

/// Composite the map into RGBA bytes: each cell's resolved atlas index, tile
/// index, flips, and palette, coloured through the frontend's DMG palette or
/// the CGB CRAM background palettes.
fn compose(map: &TileMap, graphics: &GraphicsView, colors: &ConsoleColors) -> (u32, u32, Vec<u8>) {
    let default_atlas = graphics.atlases.get(map.atlas as usize);
    let (tile_w, tile_h) = default_atlas
        .map(|atlas| (atlas.tile_width as usize, atlas.tile_height as usize))
        .unwrap_or((8, 8));
    let cols = map.columns as usize;
    let rows = map.rows as usize;
    let width = (cols * tile_w) as u32;
    let height = (rows * tile_h) as u32;

    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for tile_row in 0..rows {
        for pixel_y in 0..tile_h {
            for tile_col in 0..cols {
                let entry = map.entry(tile_col as u16, tile_row as u16);
                for pixel_x in 0..tile_w {
                    let color = entry
                        .and_then(|entry| {
                            let atlas_index = entry.atlas.unwrap_or(map.atlas) as usize;
                            let atlas = graphics.atlases.get(atlas_index)?;
                            let (sx, sy) = flipped(
                                pixel_x as u8,
                                pixel_y as u8,
                                atlas.tile_width,
                                atlas.tile_height,
                                entry.flip_x,
                                entry.flip_y,
                            );
                            let index = atlas.pixel(entry.tile as usize, sx, sy)?;
                            Some(cell_color(colors, entry.palette, index))
                        })
                        .unwrap_or(rgb::RGB8::new(0, 0, 0));
                    pixels.extend_from_slice(&[color.r, color.g, color.b, 255]);
                }
            }
        }
    }
    (width, height, pixels)
}

/// One map cell's colour: the DMG user palette, or the CGB CRAM background
/// palette the cell's attribute selects.
fn cell_color(colors: &ConsoleColors, palette: Option<u8>, index: u8) -> rgb::RGB8 {
    let index = PaletteIndex(index);
    match colors {
        ConsoleColors::Dmg { palette } => palette.color(index),
        ConsoleColors::Cgb { background, .. } => {
            background[palette.unwrap_or(0).min(7) as usize].color(index)
        }
    }
}

/// The on-screen viewports drawn over the map. A wrapping region (the GB
/// background) splits across the map edges; a non-wrapping region (the GB
/// window) anchors at the map origin and shows only the on-screen extent.
#[derive(Clone)]
struct ViewportOverlay {
    viewports: Vec<Viewport>,
    map_size: f32,
}

impl Program<app::Message> for ViewportOverlay {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let scale = bounds.width / self.map_size;

        for viewport in &self.viewports {
            let stroke = Stroke::default()
                .with_color(tone_color(viewport.tone))
                .with_width(1.5);
            let (x, y) = (viewport.x as f32, viewport.y as f32);
            let (w, h) = (viewport.width as f32, viewport.height as f32);

            if viewport.wraps {
                for (px, py, pw, ph) in wrapping_parts(x, y, w, h, self.map_size) {
                    stroke_rect(
                        &mut frame,
                        px * scale,
                        py * scale,
                        pw * scale,
                        ph * scale,
                        stroke,
                    );
                }
            } else {
                // Anchored at the map origin; (x, y) is the region's on-screen
                // offset, so only (w − x, h − y) of the map is displayed.
                let vw = (w - x).max(0.0);
                let vh = (h - y).max(0.0);
                if vw > 0.0 && vh > 0.0 {
                    stroke_rect(&mut frame, 0.0, 0.0, vw * scale, vh * scale, stroke);
                }
            }
        }

        vec![frame.into_geometry()]
    }
}

fn stroke_rect(frame: &mut Frame, x: f32, y: f32, w: f32, h: f32, stroke: Stroke) {
    frame.stroke(
        &Path::rectangle(iced::Point::new(x, y), iced::Size::new(w, h)),
        stroke,
    );
}

impl panes::Pane for TileMapPane {
    fn kind(&self) -> panes::DebuggerPane {
        panes::DebuggerPane::TileMap
    }

    fn view<'a>(
        &'a self,
        ctx: Option<&panes::PaneContext<'_>>,
        id: pane_grid::Pane,
    ) -> pane_grid::Content<'a, app::Message> {
        match (
            ctx.and_then(|ctx| ctx.graphics),
            ctx.and_then(|ctx| ctx.colors),
        ) {
            (Some(graphics), Some(colors)) => self.content(graphics, colors, id),
            _ => running_placeholder("Tile Map", id),
        }
    }

    fn on_message(&mut self, message: &panes::PaneMessage) {
        if let panes::PaneMessage::TileMap(message) = message {
            self.update(*message);
        }
    }

    fn source_index(&self) -> Option<usize> {
        Some(self.tile_map.0 as usize)
    }

    fn set_source_index(&mut self, index: usize) {
        self.tile_map = TileMapId(index as u8);
    }
}
