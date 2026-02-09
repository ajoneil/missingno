use iced::{
    Element, Length,
    widget::{container, shader},
};

use crate::{
    app::{Message, texture_renderer::TextureRenderer},
    game_boy::video::{palette::Palette, tiles::TileBlock},
};

/// Renders a grid of tiles as a single texture atlas
pub fn tile_block_atlas(block: &TileBlock, palette: &Palette) -> Element<'static, Message> {
    // 16 tiles wide × 8 tiles tall = 128 tiles total
    // Each tile is 8×8 pixels
    const ATLAS_WIDTH: u32 = 16 * 8; // 128 pixels
    const ATLAS_HEIGHT: u32 = 8 * 8; // 64 pixels
    const ATLAS_SIZE: usize = (ATLAS_WIDTH * ATLAS_HEIGHT * 4) as usize;

    let mut pixels = Vec::with_capacity(ATLAS_SIZE);

    // Render tiles row by row
    for tile_row in 0..8 {
        // For each pixel row within the tiles
        for pixel_y in 0..8 {
            // For each tile in this row
            for tile_col in 0..16 {
                let tile_index = tile_row * 16 + tile_col;
                let tile = block.tile(crate::game_boy::video::tiles::TileIndex(tile_index));

                // For each pixel in this tile's row
                for pixel_x in 0..8 {
                    let color = palette.color(tile.pixel(pixel_x, pixel_y));
                    pixels.extend_from_slice(&[color.r, color.g, color.b, 255]);
                }
            }
        }
    }

    let renderer = TextureRenderer::with_pixels(ATLAS_WIDTH, ATLAS_HEIGHT, pixels);
    container(shader(renderer).width(Length::Fill).height(Length::Fill))
        .width(Length::Fixed((ATLAS_WIDTH * 2) as f32))
        .height(Length::Fixed((ATLAS_HEIGHT * 2) as f32))
        .into()
}

/// Renders a tile map as a single texture atlas
pub fn tile_map_atlas(
    tile_map: &crate::game_boy::video::tile_maps::TileMap,
    video: &crate::game_boy::video::Video,
    palette: &Palette,
) -> Element<'static, Message> {
    // 32 tiles wide × 32 tiles tall
    // Each tile is 8×8 pixels
    const ATLAS_WIDTH: u32 = 32 * 8; // 256 pixels
    const ATLAS_HEIGHT: u32 = 32 * 8; // 256 pixels
    const ATLAS_SIZE: usize = (ATLAS_WIDTH * ATLAS_HEIGHT * 4) as usize;

    let mut pixels = Vec::with_capacity(ATLAS_SIZE);

    // Render tiles row by row
    for tile_row in 0..32 {
        // For each pixel row within the tiles
        for pixel_y in 0..8 {
            // For each tile in this row
            for tile_col in 0..32 {
                let map_tile_index = tile_map.get_tile(tile_col, tile_row);
                let (block, mapped_index) =
                    video.control().tile_address_mode().tile(map_tile_index);
                let tile = video.tile_block(block).tile(mapped_index);

                // For each pixel in this tile's row
                for pixel_x in 0..8 {
                    let color = palette.color(tile.pixel(pixel_x, pixel_y));
                    pixels.extend_from_slice(&[color.r, color.g, color.b, 255]);
                }
            }
        }
    }

    let renderer = TextureRenderer::with_pixels(ATLAS_WIDTH, ATLAS_HEIGHT, pixels);
    container(shader(renderer).width(Length::Fill).height(Length::Fill))
        .width(Length::Fixed((ATLAS_WIDTH * 2) as f32))
        .height(Length::Fixed((ATLAS_HEIGHT * 2) as f32))
        .into()
}
