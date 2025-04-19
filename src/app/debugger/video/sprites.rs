use iced::{
    Element,
    Length::Fill,
    Theme,
    widget::{Row, checkbox, column, pane_grid, radio, rich_text, row, scrollable, span},
};

use crate::{
    app::{
        Message,
        core::{
            sizes::{l, m, s, xs},
            text,
        },
        debugger::{
            panes::{DebuggerPane, checkbox_title_bar, pane},
            video::{palette::palette3, tile_widget::tile_flip},
        },
    },
    emulator::video::{
        Video,
        palette::Palette,
        sprites::{Position, Priority, Sprite, SpriteId, SpriteSize},
        tiles::{TileAddressMode, TileIndex},
    },
};

pub struct SpritesPane;

impl SpritesPane {
    pub fn new() -> Self {
        SpritesPane
    }

    pub fn content(&self, video: &Video) -> pane_grid::Content<'_, Message> {
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
                    self.sprites(video)
                ]
                .width(Fill)
                .spacing(s())
                .padding(m()),
            )
            .into(),
        )
    }

    fn sprite_size(&self, size: SpriteSize) -> Element<'static, Message> {
        row![
            text::m("Size"),
            radio(
                SpriteSize::Single.to_string(),
                SpriteSize::Single,
                Some(size),
                |_| -> Message { Message::None }
            ),
            radio(
                SpriteSize::Double.to_string(),
                SpriteSize::Double,
                Some(size),
                |_| -> Message { Message::None }
            )
        ]
        .spacing(m())
        .into()
    }

    fn sprites(&self, video: &Video) -> Element<'_, Message> {
        Row::with_children((0..40).map(|index| self.sprite(video, SpriteId(index))))
            .spacing(l())
            .wrap()
            // .vertical_spacing(m())
            .into()
    }

    fn sprite(&self, video: &Video, id: SpriteId) -> Element<'_, Message> {
        let sprite = video.sprite(id);

        row![
            checkbox("", sprite.attributes.priority() == Priority::Sprite),
            column![
                self.tiles(sprite, video),
                self.position(&sprite.position, video.control().sprite_size())
            ]
            .spacing(xs())
            .width(70)
        ]
        .into()
    }

    fn tiles(&self, sprite: &Sprite, video: &Video) -> Element<'_, Message> {
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

    fn position(&self, position: &Position, size: SpriteSize) -> Element<'_, Message> {
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
}
