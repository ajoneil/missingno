use iced::{
    Element,
    Length::Fill,
    never,
    widget::{Row, column, container, pane_grid, rich_text, row, scrollable, span, toggler, tooltip},
};

use crate::app::{
    self,
    ui::{
        fonts, palette,
        icons::{self, Icon},
        sizes::{s, xs},
    },
    debugger::{
        panes::{self, pane, title_bar_with_detail},
        ppu::tile_widget::tile_flip,
    },
};
use missingno_gb::ppu::{
    Ppu,
    memory::Vram,
    types::palette::Palette,
    types::sprites::{Position, Priority, Sprite, SpriteId, SpriteSize},
    types::tiles::{TileAddressMode, TileIndex},
};

pub struct SpritesPane {
    on_screen_only: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum Message {
    ToggleOnScreenOnly(bool),
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        panes::Message::Pane(panes::PaneMessage::Sprites(self)).into()
    }
}

impl SpritesPane {
    pub fn new() -> Self {
        SpritesPane {
            on_screen_only: true,
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::ToggleOnScreenOnly(value) => self.on_screen_only = value,
        }
    }

    pub fn content<'a>(
        &'a self,
        ppu: &'a Ppu,
        vram: &'a Vram,
        palette: &Palette,
    ) -> pane_grid::Content<'a, app::Message> {
        let size = ppu.control().sprite_size();
        let visible_count = (0..40)
            .map(|i| ppu.sprite(SpriteId(i)))
            .filter(|s| s.position.on_screen_x() && s.position.on_screen_y(size))
            .count();
        let detail = format!(
            "{} · {} visible",
            size, visible_count,
        );

        pane(
            title_bar_with_detail(
                "Sprites",
                iced::widget::text(detail)
                    .font(fonts::monospace())
                    .size(11.0)
                    .color(palette::MUTED),
            ),
            scrollable(
                column![
                    toggler(self.on_screen_only)
                        .label("On-screen only")
                        .size(14.0)
                        .on_toggle(|on| Message::ToggleOnScreenOnly(on).into()),
                    self.sprites(ppu, vram, palette)
                ]
                .width(Fill)
                .spacing(s())
                .padding(s()),
            )
            .into(),
        )
    }

    fn sprites<'a>(
        &'a self,
        ppu: &'a Ppu,
        vram: &'a Vram,
        palette: &Palette,
    ) -> Element<'a, app::Message> {
        let mut sprites = (0u8..40)
            .map(|i| (i, ppu.sprite(SpriteId(i))))
            .filter(|(_, s)| {
                if self.on_screen_only {
                    s.position.on_screen_x() && s.position.on_screen_y(ppu.control().sprite_size())
                } else {
                    true
                }
            })
            .peekable();

        if sprites.peek().is_none() {
            iced::widget::text("No on-screen sprites")
                .font(fonts::monospace())
                .size(13.0)
                .color(palette::OVERLAY0)
                .into()
        } else {
            Row::with_children(sprites.map(|(i, s)| self.sprite(i, ppu, vram, s, palette)))
                .spacing(s())
                .wrap()
                .into()
        }
    }

    fn sprite<'a>(
        &'a self,
        index: u8,
        ppu: &'a Ppu,
        vram: &'a Vram,
        sprite: &Sprite,
        palette: &Palette,
    ) -> Element<'a, app::Message> {
        let left = column![
            iced::widget::text(format!("{}", index))
                .font(fonts::monospace())
                .size(11.0)
                .color(palette::OVERLAY0),
            priority_icon(sprite.attributes.priority()),
        ]
        .spacing(xs())
        .align_x(iced::Alignment::Center);

        let right = column![
            self.tiles(sprite, vram, ppu, palette),
            self.position(&sprite.position, ppu.control().sprite_size()),
        ]
        .spacing(xs())
        .width(60);

        row![left, right]
            .spacing(xs())
            .into()
    }

    fn tiles(
        &self,
        sprite: &Sprite,
        vram: &Vram,
        ppu: &Ppu,
        palette: &Palette,
    ) -> Element<'_, app::Message> {
        let (tile_block_id, tile_id) = TileAddressMode::Block0Block1.tile(sprite.tile);
        let flip_x = sprite.attributes.flip_x();
        let flip_y = sprite.attributes.flip_y();

        match ppu.control().sprite_size() {
            SpriteSize::Single => tile_flip(
                vram.tile_block(tile_block_id).tile(tile_id),
                flip_x,
                flip_y,
                palette,
            )
            .width(40)
            .height(40)
            .into(),
            SpriteSize::Double => {
                let tile1 = tile_flip(
                    vram.tile_block(tile_block_id).tile(tile_id),
                    flip_x,
                    flip_y,
                    palette,
                )
                .width(40)
                .height(40);

                let tile2 = tile_flip(
                    vram.tile_block(tile_block_id)
                        .tile(TileIndex(tile_id.0 + 1)),
                    flip_x,
                    flip_y,
                    palette,
                )
                .width(40)
                .height(40);

                if flip_y {
                    column![tile2, tile1]
                } else {
                    column![tile1, tile2]
                }
            }
            .into(),
        }
    }

    fn position(&self, position: &Position, size: SpriteSize) -> Element<'static, app::Message> {
        rich_text![
            span(position.x_plus_8 as i16 - 8).color(if position.on_screen_x() {
                palette::GREEN
            } else {
                palette::RED
            }),
            span(",").color(palette::MUTED),
            span(position.y_plus_16 as i16 - 16).color(if position.on_screen_y(size) {
                palette::GREEN
            } else {
                palette::RED
            }),
        ]
        .font(fonts::monospace())
        .size(13.0)
        .on_link_click(never)
        .into()
    }
}

fn priority_icon(priority: Priority) -> Element<'static, app::Message> {
    use crate::app::debugger::sidebar::tooltip_style;

    let (icon, label) = match priority {
        Priority::Sprite => (Icon::Front, "Above BG"),
        Priority::Background => (Icon::Back, "Behind BG"),
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
