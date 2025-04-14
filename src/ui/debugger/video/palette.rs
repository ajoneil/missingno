use crate::{
    emulator::video::palette::{Palette, PaletteIndex, PaletteMap},
    ui::Message,
};

use super::iced_color;
use iced::{
    Border, Color, Element, Length,
    border::Radius,
    widget::{container, row},
};

pub fn palette4(map: &PaletteMap, palette: &Palette) -> Element<'static, Message> {
    container(
        row![
            color_block(map, PaletteIndex(0), palette),
            color_block(map, PaletteIndex(1), palette),
            color_block(map, PaletteIndex(2), palette),
            color_block(map, PaletteIndex(3), palette),
        ]
        .spacing(5),
    )
    .height(20)
    .into()
}

// Sprites always treat color #0 as transparent
pub fn palette3(map: &PaletteMap, palette: &Palette) -> Element<'static, Message> {
    container(
        row![
            color_block(map, PaletteIndex(1), palette),
            color_block(map, PaletteIndex(2), palette),
            color_block(map, PaletteIndex(3), palette),
        ]
        .spacing(3),
    )
    .height(20)
    .into()
}

fn color_block(
    map: &PaletteMap,
    index: PaletteIndex,
    palette: &Palette,
) -> Element<'static, Message> {
    let c = iced_color(map.color(index, &palette));
    container("")
        .style(move |_| {
            container::background(c).border(Border {
                color: Color::BLACK,
                width: 1.0,
                radius: Radius::new(5.0),
            })
        })
        .height(Length::Fill)
        .width(50)
        .into()
}
