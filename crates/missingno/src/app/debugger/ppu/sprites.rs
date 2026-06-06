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
        panes::{self, pane, title_bar_with_detail},
        ppu::tile_widget::tile_flip,
    },
    ui::{
        fonts,
        icons::{self, Icon},
        palette,
        sizes::{s, xs},
    },
};
use missingno_gb::ppu::{
    Ppu,
    memory::{Vram, VramBank},
    model::PpuModel,
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

    pub fn content<'a, P: PpuModel>(
        &'a self,
        ppu: &'a Ppu<P>,
        vram: &'a P::Vram,
        colors: &ConsoleColors,
    ) -> pane_grid::Content<'a, app::Message> {
        let size = ppu.control().sprite_size();
        let visible_count = (0..40)
            .map(|i| ppu.sprite(SpriteId(i)))
            .filter(|s| s.position.on_screen_x() && s.position.on_screen_y(size))
            .count();
        let detail = format!("{} · {} visible", size, visible_count,);

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
                    self.sprites(ppu, vram, colors)
                ]
                .width(Fill)
                .spacing(s())
                .padding(s()),
            )
            .into(),
        )
    }

    fn sprites<'a, P: PpuModel>(
        &'a self,
        ppu: &'a Ppu<P>,
        vram: &'a P::Vram,
        colors: &ConsoleColors,
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
            Row::with_children(sprites.map(|(i, s)| self.sprite(i, ppu, vram, s, colors)))
                .spacing(s())
                .wrap()
                .into()
        }
    }

    fn sprite<'a, P: PpuModel>(
        &'a self,
        index: u8,
        ppu: &'a Ppu<P>,
        vram: &'a P::Vram,
        sprite: &Sprite,
        colors: &ConsoleColors,
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
            self.tiles(sprite, vram, ppu, colors),
            self.position(&sprite.position, ppu.control().sprite_size()),
        ]
        .spacing(xs())
        .width(60);

        row![left, right].spacing(xs()).into()
    }

    fn tiles<P: PpuModel>(
        &self,
        sprite: &Sprite,
        vram: &P::Vram,
        ppu: &Ppu<P>,
        colors: &ConsoleColors,
    ) -> Element<'_, app::Message> {
        // CGB sprites carry their CRAM palette and tile bank in OAM attributes.
        let (bank, palette): (&VramBank, &Palette) = match colors {
            ConsoleColors::Dmg { palette } => (vram.bank(0), palette),
            ConsoleColors::Cgb { objects, .. } => (
                vram.bank(sprite.attributes.cgb_bank()),
                &objects[sprite.attributes.cgb_palette() as usize],
            ),
        };

        let flip_x = sprite.attributes.flip_x();
        let flip_y = sprite.attributes.flip_y();

        let sprite_tile = |index: TileIndex| {
            let (block, tile) = TileAddressMode::Block0Block1.tile(index);
            tile_flip(bank.tile_block(block).tile(tile), flip_x, flip_y, palette)
                .width(40)
                .height(40)
        };

        match ppu.control().sprite_size() {
            SpriteSize::Single => sprite_tile(sprite.tile).into(),
            SpriteSize::Double => {
                // Hardware ignores bit 0 of the index: top is tile&FE, bottom tile|01.
                let tile1 = sprite_tile(TileIndex(sprite.tile.0 & 0xFE));
                let tile2 = sprite_tile(TileIndex(sprite.tile.0 | 0x01));

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
            span(position.x as i16 - 8).color(if position.on_screen_x() {
                palette::GREEN
            } else {
                palette::RED
            }),
            span(",").color(palette::MUTED),
            span(position.y as i16 - 16).color(if position.on_screen_y(size) {
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
