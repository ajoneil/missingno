use iced::widget::shader;

use crate::app::{Message, texture_renderer::TextureRenderer};
use missingno_core::game_boy::video::{palette::Palette, tiles::Tile};

pub fn tile_flip(
    tile: Tile,
    flip_x: bool,
    flip_y: bool,
    palette: &Palette,
) -> iced::widget::Shader<Message, TextureRenderer> {
    let mut pixels = Vec::with_capacity(8 * 8 * 4);

    for y in 0..8 {
        for x in 0..8 {
            let read_x = if flip_x { 7 - x } else { x };
            let read_y = if flip_y { 7 - y } else { y };

            let color = palette.color(tile.pixel(read_x, read_y));
            pixels.extend_from_slice(&[color.r, color.g, color.b, 255]);
        }
    }

    let renderer = TextureRenderer::with_pixels(8, 8, pixels);
    shader(renderer).width(24).height(24)
}
