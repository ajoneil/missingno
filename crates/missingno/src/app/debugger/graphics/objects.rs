//! The object table pane: a family's sprite/OAM entries as cards — each an
//! index, a priority indicator, the composed thumbnail (single tile or the
//! 8×16 two-tile stack, with flips applied), and the screen-space position.
//! An "on-screen only" toggle hides objects parked off the visible area.

use iced::{
    Element,
    Length::Fill,
    never,
    widget::{
        Row, column, container, pane_grid, rich_text, row, scrollable, span, toggler, tooltip,
    },
};

use crate::app::{
    self,
    console::ConsoleColors,
    debugger::{
        graphics::{flipped, stacked_tiles},
        panes::{self, pane, running_placeholder, title_bar_with_detail},
    },
    texture_renderer::TextureRenderer,
    ui::{
        fonts,
        icons::{self, Icon},
        palette,
        sizes::{s, xs},
    },
};
use missingno_core::graphics::{GraphicsView, Object, ObjectTable, TileAtlas};
use missingno_gb::ppu::types::palette::PaletteIndex;

/// Thumbnail pixels per source pixel.
const THUMB_SCALE: u32 = 5;

pub struct ObjectTablePane {
    on_screen_only: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum Message {
    ToggleOnScreenOnly(bool),
}

impl From<Message> for app::Message {
    fn from(val: Message) -> Self {
        panes::Message::Broadcast(panes::PaneMessage::Sprites(val)).into()
    }
}

impl ObjectTablePane {
    pub fn new() -> Self {
        Self {
            on_screen_only: true,
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::ToggleOnScreenOnly(value) => self.on_screen_only = value,
        }
    }

    fn content<'a>(
        &'a self,
        graphics: &GraphicsView,
        colors: &ConsoleColors,
        close: pane_grid::Pane,
    ) -> pane_grid::Content<'a, app::Message> {
        let Some(table) = &graphics.objects else {
            return running_placeholder("Sprites", close);
        };

        let height = table.object_height;
        let size = format!("8×{height}");
        let visible = table
            .objects
            .iter()
            .filter(|object| object.on_screen)
            .count();
        let detail = format!("{size} · {visible} visible");

        let cards = table
            .objects
            .iter()
            .filter(|object| !self.on_screen_only || object.on_screen)
            .filter_map(|object| card(object, graphics, table, colors));
        let cards: Vec<Element<'a, app::Message>> = cards.collect();

        let body: Element<'a, app::Message> = if cards.is_empty() {
            iced::widget::text("No on-screen sprites")
                .font(fonts::monospace())
                .size(13.0)
                .color(palette::OVERLAY0)
                .into()
        } else {
            Row::with_children(cards).spacing(s()).wrap().into()
        };

        pane(
            title_bar_with_detail(
                "Sprites",
                iced::widget::text(detail)
                    .font(fonts::monospace())
                    .size(11.0)
                    .color(palette::MUTED),
                close,
            ),
            scrollable(
                column![
                    toggler(self.on_screen_only)
                        .label("On-screen only")
                        .size(14.0)
                        .on_toggle(|on| Message::ToggleOnScreenOnly(on).into()),
                    body,
                ]
                .width(Fill)
                .spacing(s())
                .padding(s()),
            )
            .into(),
        )
    }
}

/// One object's card: index and priority icon beside the thumbnail and its
/// screen-space position. `None` when the object's pattern atlas is absent.
fn card<'a>(
    object: &Object,
    graphics: &GraphicsView,
    table: &ObjectTable,
    colors: &ConsoleColors,
) -> Option<Element<'a, app::Message>> {
    let atlas = graphics
        .atlases
        .get(object.bank.unwrap_or(table.atlas) as usize)?;

    let left = column![
        iced::widget::text(format!("{}", object.index))
            .font(fonts::monospace())
            .size(11.0)
            .color(palette::OVERLAY0),
        priority_icon(object.priority),
    ]
    .spacing(xs())
    .align_x(iced::Alignment::Center);

    let right = column![thumbnail(object, atlas, table, colors), position(object)]
        .spacing(xs())
        .width(60);

    Some(row![left, right].spacing(xs()).into())
}

/// The object thumbnail: a single 8×8 tile or an 8×16 two-tile stack, flips
/// applied, coloured through the DMG user palette or the CGB CRAM OBJ palette.
fn thumbnail<'a>(
    object: &Object,
    atlas: &TileAtlas,
    table: &ObjectTable,
    colors: &ConsoleColors,
) -> Element<'a, app::Message> {
    let tile_w = atlas.tile_width;
    let tile_h = atlas.tile_height;

    let slots: Vec<u16> = if table.object_height > tile_h {
        let (top, bottom) = stacked_tiles(object.tile, object.flip_y);
        vec![top, bottom]
    } else {
        vec![object.tile]
    };

    let width = tile_w as u32;
    let height = tile_h as u32 * slots.len() as u32;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for slot in slots {
        for y in 0..tile_h {
            for x in 0..tile_w {
                let (sx, sy) = flipped(x, y, tile_w, tile_h, object.flip_x, object.flip_y);
                let index = atlas.pixel(slot as usize, sx, sy).unwrap_or(0);
                let color = object_color(colors, object.palette, index);
                pixels.extend_from_slice(&[color.r, color.g, color.b, 255]);
            }
        }
    }

    let renderer = TextureRenderer::with_pixels(width, height, pixels);
    iced::widget::shader(renderer)
        .width((width * THUMB_SCALE) as f32)
        .height((height * THUMB_SCALE) as f32)
        .into()
}

/// An object pixel's colour: the DMG user palette, or the CGB CRAM OBJ palette
/// the object's attribute selects.
fn object_color(colors: &ConsoleColors, palette: Option<u8>, index: u8) -> rgb::RGB8 {
    let index = PaletteIndex(index);
    match colors {
        ConsoleColors::Dmg { palette } => palette.color(index),
        ConsoleColors::Cgb { objects, .. } => {
            objects[palette.unwrap_or(0).min(7) as usize].color(index)
        }
    }
}

fn position<'a>(object: &Object) -> Element<'a, app::Message> {
    let tint = if object.on_screen {
        palette::GREEN
    } else {
        palette::RED
    };
    rich_text![
        span(object.x).color(tint),
        span(",").color(palette::MUTED),
        span(object.y).color(tint),
    ]
    .font(fonts::monospace())
    .size(13.0)
    .on_link_click(never)
    .into()
}

fn priority_icon<'a>(behind_background: bool) -> Element<'a, app::Message> {
    use crate::app::debugger::sidebar::tooltip_style;

    let (icon, label) = if behind_background {
        (Icon::Back, "Behind BG")
    } else {
        (Icon::Front, "Above BG")
    };

    tooltip(
        icons::m_muted(icon),
        container(
            iced::widget::text(label)
                .font(fonts::monospace())
                .size(11.0),
        )
        .padding([2.0, s()]),
        tooltip::Position::Right,
    )
    .style(tooltip_style)
    .into()
}

impl panes::Pane for ObjectTablePane {
    fn kind(&self) -> panes::DebuggerPane {
        panes::DebuggerPane::Sprites
    }

    fn view<'a>(
        &'a self,
        ctx: Option<&panes::PaneContext<'_>>,
        id: pane_grid::Pane,
    ) -> pane_grid::Content<'a, app::Message> {
        match (ctx.and_then(|ctx| ctx.graphics), ctx.and_then(|ctx| ctx.gb)) {
            (Some(graphics), Some(gb)) => self.content(graphics, gb.colors, id),
            _ => running_placeholder("Sprites", id),
        }
    }

    fn on_message(&mut self, message: &panes::PaneMessage) {
        if let panes::PaneMessage::Sprites(message) = message {
            self.update(*message);
        }
    }
}
