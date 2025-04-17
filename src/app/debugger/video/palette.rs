use iced::{
    Border, Color, Element, Length,
    border::Radius,
    widget::{container, row},
};

use crate::{
    app::{
        Message,
        core::sizes::{m, s, xs},
        screen::iced_color,
    },
    emulator::video::palette::{Palette, PaletteIndex, PaletteMap},
};

pub fn palette4(map: &PaletteMap, palette: &Palette) -> Element<'static, Message> {
    container(
        row![
            color_block(map, PaletteIndex(0), palette),
            color_block(map, PaletteIndex(1), palette),
            color_block(map, PaletteIndex(2), palette),
            color_block(map, PaletteIndex(3), palette),
        ]
        .spacing(xs()),
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
        .spacing(xs()),
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
                radius: Radius::new(s()),
            })
        })
        .height(Length::Fill)
        .width(m() * 3.0)
        .into()
}
