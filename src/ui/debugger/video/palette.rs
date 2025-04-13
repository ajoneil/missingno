use crate::{
    emulator::video::palette::{Palette, PaletteMap},
    ui::Message,
};

use iced::{
    Border, Color, Element, Length,
    border::Radius,
    widget::{container, row},
};
use rgb::RGB8;

pub fn palette4(map: &PaletteMap, palette: &Palette) -> Element<'static, Message> {
    container(
        row![
            color_block(map, 0, palette),
            color_block(map, 1, palette),
            color_block(map, 2, palette),
            color_block(map, 3, palette),
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
            color_block(map, 1, palette),
            color_block(map, 2, palette),
            color_block(map, 3, palette),
        ]
        .spacing(3),
    )
    .height(20)
    .into()
}

fn color_block(map: &PaletteMap, index: u8, palette: &Palette) -> Element<'static, Message> {
    let c = iced_color(map.get(index, &palette));
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

fn iced_color(color: RGB8) -> Color {
    Color::from_rgb8(color.r, color.g, color.b)
}
