use crate::{
    emulator::video::{
        control::Control, sprite::SpriteSize, tile::TileAddressMode, tile_map::TileMapRegion,
    },
    ui::Message,
};

use iced::{
    Element,
    widget::{checkbox, column, horizontal_rule, radio, row, text},
};

pub fn control(control: Control) -> Element<'static, Message> {
    column![
        general(control),
        horizontal_rule(1),
        window(control),
        horizontal_rule(1),
        background(control),
        horizontal_rule(1),
        sprites(control)
    ]
    .spacing(5)
    .into()
}

fn general(control: Control) -> Element<'static, Message> {
    column![
        row![checkbox("Video Enabled", control.video_enabled()),].spacing(10),
        tile_address_mode(control)
    ]
    .spacing(5)
    .into()
}

fn window(control: Control) -> Element<'static, Message> {
    column![
        checkbox("Window Enabled", control.window_enabled()),
        tile_map_region("Window Tile Map", control.window_tile_map_region())
    ]
    .spacing(5)
    .into()
}

fn background(control: Control) -> Element<'static, Message> {
    column![
        checkbox(
            "Background & Window Enabled",
            control.background_and_window_enabled()
        ),
        tile_map_region("Background Tile Map", control.background_tile_map_region())
    ]
    .spacing(5)
    .into()
}

fn sprites(control: Control) -> Element<'static, Message> {
    column![
        checkbox("Sprites enabled", control.sprites_enabled()),
        sprite_size(control.sprite_size())
    ]
    .spacing(5)
    .into()
}

fn tile_address_mode(control: Control) -> Element<'static, Message> {
    row![
        text("Tiles"),
        radio(
            TileAddressMode::Block0Block1.to_string(),
            TileAddressMode::Block0Block1,
            Some(control.tile_address_mode()),
            |_| -> Message { Message::None }
        ),
        radio(
            TileAddressMode::Block2Block1.to_string(),
            TileAddressMode::Block2Block1,
            Some(control.tile_address_mode()),
            |_| -> Message { Message::None }
        )
    ]
    .spacing(10)
    .into()
}

fn tile_map_region(label: &str, region: TileMapRegion) -> Element<'_, Message> {
    row![
        text(label),
        radio(
            TileMapRegion::Map9800.to_string(),
            TileMapRegion::Map9800,
            Some(region),
            |_| -> Message { Message::None }
        ),
        radio(
            TileMapRegion::Map9c00.to_string(),
            TileMapRegion::Map9c00,
            Some(region),
            |_| -> Message { Message::None }
        )
    ]
    .spacing(10)
    .into()
}

fn sprite_size(size: SpriteSize) -> Element<'static, Message> {
    row![
        text("Sprite Size"),
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
    .spacing(10)
    .into()
}
