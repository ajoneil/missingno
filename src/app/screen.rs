use iced::{Color, Point, Renderer, Size, Theme, advanced::mouse, widget::canvas};
use rgb::RGB8;

use crate::game_boy::video::{
    palette::Palette,
    screen::{self, Screen},
};

impl<Message> canvas::Program<Message> for Screen {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let size = bounds.size().height.min(bounds.size().width);
        let pixel_size = Size::new(
            size / screen::PIXELS_PER_LINE as f32,
            size / screen::NUM_SCANLINES as f32,
        );
        let mut frame = canvas::Frame::new(renderer, Size::new(size, size));

        for x in 0..screen::PIXELS_PER_LINE {
            for y in 0..screen::NUM_SCANLINES {
                frame.fill_rectangle(
                    Point::new(
                        x as f32 * size / screen::PIXELS_PER_LINE as f32,
                        y as f32 * size / screen::NUM_SCANLINES as f32,
                    ),
                    pixel_size,
                    iced_color(Palette::MONOCHROME_GREEN.color(self.pixel(x, y))),
                );
            }
        }

        vec![frame.into_geometry()]
    }
}

pub fn iced_color(color: RGB8) -> Color {
    Color::from_rgb8(color.r, color.g, color.b)
}
