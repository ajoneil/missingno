use iced::{
    Length,
    Length::Fill,
    widget::{container, pane_grid, responsive, shader, Stack},
};

use crate::app::{
    Message,
    debugger::panes::{pane, title_bar},
    texture_renderer::TextureRenderer,
};
use missingno_gb::ppu::{
    Ppu, memory::Vram, types::control::Control, types::palette::Palette,
    types::tile_maps::TileMapId,
};

mod viewport_overlay;

pub struct TileMapPane {
    tile_map: TileMapId,
    title: String,
}

impl TileMapPane {
    pub fn new(tile_map: TileMapId) -> Self {
        Self {
            tile_map,
            title: tile_map.to_string(),
        }
    }

    pub fn content(
        &self,
        ppu: &Ppu,
        vram: &Vram,
        palette: &Palette,
    ) -> pane_grid::Content<'_, Message> {
        let control = ppu.control();
        let tile_map_id = self.tile_map;

        let scx = ppu.read_register(missingno_gb::ppu::Register::BackgroundViewportX);
        let scy = ppu.read_register(missingno_gb::ppu::Register::BackgroundViewportY);
        let wx = ppu.read_register(missingno_gb::ppu::Register::WindowX);
        let wy = ppu.read_register(missingno_gb::ppu::Register::WindowY);

        let bg_viewport = if tile_map_id == control.background_tile_map() {
            Some((scx, scy))
        } else {
            None
        };
        let win_viewport = if tile_map_id == control.window_tile_map() {
            Some((wx, wy))
        } else {
            None
        };

        // Pre-render tile map pixels so the closure doesn't need Vram
        let pixels = render_tile_map(vram.tile_map(tile_map_id), control, vram, palette);

        let overlay = viewport_overlay::ViewportOverlay {
            bg_viewport,
            win_viewport,
        };

        pane(
            title_bar(&self.title),
            responsive(move |size| {
                let fit = size.width.min(size.height);

                let renderer = TextureRenderer::with_pixels(256, 256, pixels.clone());
                let overlay = overlay.clone();

                container(
                    Stack::new()
                        .push(
                            shader(renderer)
                                .width(Length::Fixed(fit))
                                .height(Length::Fixed(fit)),
                        )
                        .push(
                            iced::widget::canvas(overlay)
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
}

/// Pre-render tile map pixels as RGBA bytes.
fn render_tile_map(
    tile_map: &missingno_gb::ppu::types::tile_maps::TileMap,
    control: Control,
    vram: &Vram,
    palette: &Palette,
) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(256 * 256 * 4);

    for tile_row in 0..32 {
        for pixel_y in 0..8 {
            for tile_col in 0..32 {
                let map_tile_index = tile_map.get_tile(tile_col, tile_row);
                let (block, mapped_index) = control.tile_address_mode().tile(map_tile_index);
                let tile = vram.tile_block(block).tile(mapped_index);

                for pixel_x in 0..8 {
                    let color = palette.color(tile.pixel(pixel_x, pixel_y));
                    pixels.extend_from_slice(&[color.r, color.g, color.b, 255]);
                }
            }
        }
    }

    pixels
}
