use iced::{
    Length,
    Length::Fill,
    widget::{Stack, container, pane_grid, responsive, shader},
};

use crate::app::{
    Message,
    console::ConsoleColors,
    debugger::panes::{pane, title_bar},
    texture_renderer::TextureRenderer,
};
use crate::render::{tile_map_rgba, tile_map_rgba_cgb};
use missingno_gb::ppu::{Ppu, memory::Vram, model::PpuModel, types::tiles::TileMapId};

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

    pub fn content<P: PpuModel>(
        &self,
        ppu: &Ppu<P>,
        vram: &P::Vram,
        colors: &ConsoleColors,
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

        // Pre-render tile map pixels so the closure doesn't need VramBank
        let pixels: std::sync::Arc<[u8]> = match colors {
            ConsoleColors::Dmg { palette } => {
                tile_map_rgba(vram.bank(0), tile_map_id, control, palette)
            }
            ConsoleColors::Cgb { background, .. } => {
                tile_map_rgba_cgb(vram, tile_map_id, control, background)
            }
        }
        .into();

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
