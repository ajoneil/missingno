use iced::widget::shader;
use rgb::RGB8;

use crate::game_boy::video::{
    palette::{Palette, PaletteChoice, PaletteIndex},
    screen::{self, Screen},
};

use super::texture_renderer::TextureRenderer;

pub struct ScreenView {
    pub screen: Screen,
    pub palette: PaletteChoice,
}

impl ScreenView {
    pub fn new() -> Self {
        Self {
            screen: Screen::new(),
            palette: PaletteChoice::default(),
        }
    }
}

impl<Message> shader::Program<Message> for ScreenView {
    type State = ();
    type Primitive = <TextureRenderer as shader::Program<Message>>::Primitive;

    fn draw(
        &self,
        _state: &Self::State,
        cursor: iced::mouse::Cursor,
        bounds: iced::Rectangle,
    ) -> Self::Primitive {
        let pixels = screen_to_pixels(&self.screen, self.palette.palette());
        let renderer = TextureRenderer::with_pixels(
            screen::PIXELS_PER_LINE as u32,
            screen::NUM_SCANLINES as u32,
            pixels,
        );

        <TextureRenderer as shader::Program<Message>>::draw(&renderer, &(), cursor, bounds)
    }
}

pub fn screen_to_pixels(screen: &Screen, palette: &Palette) -> Vec<u8> {
    let mut pixels =
        Vec::with_capacity(screen::PIXELS_PER_LINE as usize * screen::NUM_SCANLINES as usize * 4);

    for y in 0..screen::NUM_SCANLINES {
        for x in 0..screen::PIXELS_PER_LINE {
            let color = palette.color(screen.pixel(x, y));
            pixels.extend_from_slice(&[color.r, color.g, color.b, 255]);
        }
    }

    pixels
}

pub fn iced_color(color: RGB8) -> iced::Color {
    iced::Color::from_rgb8(color.r, color.g, color.b)
}

pub fn palette_swatch<Message: 'static>(palette: &Palette) -> iced::Element<'static, Message> {
    use super::core::sizes::{s, xs};
    use iced::{Border, Color, Length, border::Radius, widget::container};

    let mut row = iced::widget::Row::new().spacing(xs());

    for i in 0..4 {
        let c = iced_color(palette.color(PaletteIndex(i)));
        row = row.push(
            container("")
                .style(move |_| {
                    container::background(c).border(Border {
                        color: Color::BLACK,
                        width: 1.0,
                        radius: Radius::new(s()),
                    })
                })
                .height(Length::Fill)
                .width(16.0),
        );
    }

    container(row).height(16).into()
}
