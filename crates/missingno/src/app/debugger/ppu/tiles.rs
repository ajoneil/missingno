use iced::{
    Element,
    Length::Fill,
    widget::{column, pane_grid, row, scrollable, text, toggler},
};

use crate::app::{
    self,
    console::ConsoleColors,
    debugger::{
        panes::{self, pane, title_bar, title_bar_with_detail},
        ppu::tile_atlas::tile_block_atlas,
    },
    ui::sizes::m,
};
use missingno_gb::ppu::{
    memory::{Vram, VramBank},
    types::palette::Palette,
    types::tiles::TileBlockId,
};

pub struct TilesPane {
    selected_bank: u8,
}

#[derive(Debug, Clone, Copy)]
pub enum Message {
    SelectBank(u8),
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        panes::Message::Pane(panes::PaneMessage::Tiles(self)).into()
    }
}

impl TilesPane {
    pub fn new() -> Self {
        Self { selected_bank: 0 }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::SelectBank(bank) => self.selected_bank = bank,
        }
    }

    pub fn content(
        &self,
        vram: &impl Vram,
        colors: &ConsoleColors,
    ) -> pane_grid::Content<'_, app::Message> {
        let palette = colors.tiles_palette();
        let bank = vram.bank(self.selected_bank);

        let title = if colors.is_cgb() {
            title_bar_with_detail(
                "Tiles",
                toggler(self.selected_bank == 1)
                    .label("bank 1")
                    .size(14.0)
                    .on_toggle(|on| Message::SelectBank(on as u8).into()),
            )
        } else {
            title_bar("Tiles")
        };

        pane(
            title,
            scrollable(
                row![
                    tile_block(bank, TileBlockId(0), palette),
                    tile_block(bank, TileBlockId(1), palette),
                    tile_block(bank, TileBlockId(2), palette)
                ]
                .spacing(m())
                .padding(m())
                .width(Fill)
                .wrap(),
            )
            .into(),
        )
    }
}

fn tile_block(
    vram: &VramBank,
    block: TileBlockId,
    palette: &Palette,
) -> Element<'static, app::Message> {
    column![
        text(block.to_string()),
        tile_block_atlas(vram.tile_block(block), palette)
    ]
    .into()
}
