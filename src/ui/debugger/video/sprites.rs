use iced::{
    Element,
    widget::{checkbox, column, radio, row, text},
};

use super::palette::palette3;
use crate::{
    emulator::video::{Video, palette::Palette, sprites::SpriteSize},
    ui::{Message, styles::spacing},
};

pub fn sprites(video: &Video) -> Element<'static, Message> {
    column![
        checkbox("Sprites", video.control().sprites_enabled()),
        sprite_size(video.control().sprite_size()),
        row![
            text("Palette 0"),
            palette3(&video.palettes().sprite0, &Palette::MONOCHROME_GREEN)
        ]
        .spacing(spacing::m()),
        row![
            text("Palette 1"),
            palette3(&video.palettes().sprite1, &Palette::MONOCHROME_GREEN)
        ]
        .spacing(spacing::m())
    ]
    .spacing(spacing::s())
    .into()
}

fn sprite_size(size: SpriteSize) -> Element<'static, Message> {
    row![
        text("Size"),
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
    .spacing(spacing::m())
    .into()
}
