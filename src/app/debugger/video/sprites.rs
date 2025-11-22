use iced::{
    Element,
    Length::Fill,
    Theme,
    widget::{Row, column, pane_grid, radio, rich_text, row, scrollable, span, toggler},
};

use crate::{
    app::{
        self,
        core::{
            icons::{self, Icon},
            sizes::{l, m, s, xs},
            text,
        },
        debugger::{
            panes::{self, DebuggerPane, checkbox_title_bar, pane},
            video::{palette::palette3, tile_widget::tile_flip},
        },
    },
    game_boy::video::{
        Video,
        palette::Palette,
        sprites::{Position, Priority, Sprite, SpriteId, SpriteSize},
        tiles::{TileAddressMode, TileIndex},
    },
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

    pub fn content(&self, video: &Video) -> pane_grid::Content<'_, app::Message> {
        pane(
            checkbox_title_bar(
                "Sprites",
                video.control().sprites_enabled(),
                DebuggerPane::Sprites,
            ),
            scrollable(
                column![
                    self.sprite_size(video.control().sprite_size()),
                    row![
                        text::m("Palette 0"),
                        palette3(&video.palettes().sprite0, &Palette::MONOCHROME_GREEN)
                    ]
                    .spacing(m()),
                    row![
                        text::m("Palette 1"),
                        palette3(&video.palettes().sprite1, &Palette::MONOCHROME_GREEN)
                    ]
                    .spacing(m()),
                    toggler(self.on_screen_only)
                        .label("Show on-screen only")
                        .on_toggle(|on| Message::ToggleOnScreenOnly(on).into()),
                    self.sprites(video)
                ]
                .width(Fill)
                .spacing(s())
                .padding(m()),
            )
            .into(),
        )
    }

    fn sprite_size(&self, size: SpriteSize) -> Element<'static, app::Message> {
        row![
            text::m("Size"),
            radio(
                SpriteSize::Single.to_string(),
                SpriteSize::Single,
                Some(size),
                |_| -> app::Message { app::Message::None }
            ),
            radio(
                SpriteSize::Double.to_string(),
                SpriteSize::Double,
                Some(size),
                |_| -> app::Message { app::Message::None }
            )
        ]
        .spacing(m())
        .into()
    }

    fn sprites(&self, video: &Video) -> Element<'_, app::Message> {
        let mut sprites = (0..40)
            .map(|i| video.sprite(SpriteId(i)))
            .filter(|s| {
                if self.on_screen_only {
                    s.position.on_screen_x()
                        && s.position.on_screen_y(video.control().sprite_size())
                } else {
                    true
                }
            })
            .peekable();

        if sprites.peek().is_none() {
            text::m("No on-screen sprites").into()
        } else {
            Row::with_children(sprites.map(|s| self.sprite(video, s)))
                .spacing(l())
                .wrap()
                .into()
        }
    }

    fn sprite(&self, video: &Video, sprite: &Sprite) -> Element<'_, app::Message> {
        row![
            self.priority(sprite.attributes.priority()),
            column![
                self.tiles(sprite, video),
                self.position(&sprite.position, video.control().sprite_size())
            ]
            .spacing(xs())
            .width(60)
        ]
        .spacing(s())
        .into()
    }

    fn tiles(&self, sprite: &Sprite, video: &Video) -> Element<'_, app::Message> {
        let (tile_block_id, tile_id) = TileAddressMode::Block0Block1.tile(sprite.tile);
        let flip_x = sprite.attributes.flip_x();
        let flip_y = sprite.attributes.flip_y();

        match video.control().sprite_size() {
            SpriteSize::Single => tile_flip(
                video.tile_block(tile_block_id).tile(tile_id),
                flip_x,
                flip_y,
            )
            .width(48)
            .height(48)
            .into(),
            SpriteSize::Double => {
                let tile1 = tile_flip(
                    video.tile_block(tile_block_id).tile(tile_id),
                    flip_x,
                    flip_y,
                )
                .width(48)
                .height(48);

                let tile2 = tile_flip(
                    video
                        .tile_block(tile_block_id)
                        .tile(TileIndex(tile_id.0 + 1)),
                    flip_x,
                    flip_y,
                )
                .width(48)
                .height(48);

                if flip_y {
                    column![tile2, tile1]
                } else {
                    column![tile1, tile2]
                }
            }
            .into(),
        }
    }

    fn position(&self, position: &Position, size: SpriteSize) -> Element<'_, app::Message> {
        let visible = Theme::CatppuccinMocha.palette().success;
        let offscreen = Theme::CatppuccinMocha.palette().danger;

        rich_text([
            span(position.x_plus_8 as i16 - 8).color(if position.on_screen_x() {
                visible
            } else {
                offscreen
            }),
            span(","),
            span(position.y_plus_16 as i16 - 16).color(if position.on_screen_y(size) {
                visible
            } else {
                offscreen
            }),
        ])
        .into()
    }

    fn priority(&self, priority: Priority) -> Element<'_, app::Message> {
        icons::m(match priority {
            Priority::Sprite => Icon::Front,
            Priority::Background => Icon::Back,
        })
        .into()
    }
}
