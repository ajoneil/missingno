use iced::{Element, Point, Renderer, Size, Theme, mouse, widget::canvas};

use crate::{
    app::{Message, screen::iced_color},
    emulator::video::{palette::Palette, tiles::Tile},
};

pub fn tile(tile: Tile) -> Element<'static, Message> {
    canvas(tile).width(24).height(24).into()
}

impl<Message> canvas::Program<Message> for Tile {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let widget_size = bounds.size();
        let pixel_size = Size::new(widget_size.width / 8.0, widget_size.height / 8.0);
        let mut frame = canvas::Frame::new(renderer, widget_size);

        for x in 0..8 {
            for y in 0..8 {
                frame.fill_rectangle(
                    Point::new(
                        x as f32 * widget_size.width / 8.0,
                        y as f32 * widget_size.height / 8.0,
                    ),
                    pixel_size,
                    iced_color(Palette::MONOCHROME_GREEN.color(self.pixel(x, y))),
                );
            }
        }

        vec![frame.into_geometry()]
    }
}
