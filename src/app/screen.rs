use iced::widget::shader;
use rgb::RGB8;

use crate::game_boy::video::{
    palette::Palette,
    screen::{self, Screen},
};

use super::texture_renderer::TextureRenderer;

impl<Message> shader::Program<Message> for Screen {
    type State = ();
    type Primitive = <TextureRenderer as shader::Program<Message>>::Primitive;

    fn draw(
        &self,
        _state: &Self::State,
        cursor: iced::mouse::Cursor,
        bounds: iced::Rectangle,
    ) -> Self::Primitive {
        let pixels = screen_to_pixels(self);
        let renderer = TextureRenderer::with_pixels(
            screen::PIXELS_PER_LINE as u32,
            screen::NUM_SCANLINES as u32,
            pixels,
        );

        <TextureRenderer as shader::Program<Message>>::draw(&renderer, &(), cursor, bounds)
    }
}

fn screen_to_pixels(screen: &Screen) -> Vec<u8> {
    let mut pixels =
        Vec::with_capacity(screen::PIXELS_PER_LINE as usize * screen::NUM_SCANLINES as usize * 4);

    for y in 0..screen::NUM_SCANLINES {
        for x in 0..screen::PIXELS_PER_LINE {
            let color = Palette::MONOCHROME_GREEN.color(screen.pixel(x, y));
            pixels.extend_from_slice(&[color.r, color.g, color.b, 255]);
        }
    }

    pixels
}

pub fn iced_color(color: RGB8) -> iced::Color {
    iced::Color::from_rgb8(color.r, color.g, color.b)
}
