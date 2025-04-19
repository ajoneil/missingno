use iced::{
    Point, Renderer, Size, Theme, mouse,
    widget::{Canvas, canvas},
};

use crate::{
    app::{Message, screen::iced_color},
    emulator::video::{palette::Palette, tiles::Tile},
};

pub fn tile(tile: Tile) -> Canvas<RenderTile, Message> {
    canvas(RenderTile {
        tile,
        flip_x: false,
        flip_y: false,
    })
    .width(24)
    .height(24)
}

pub fn tile_flip(tile: Tile, flip_x: bool, flip_y: bool) -> Canvas<RenderTile, Message> {
    canvas(RenderTile {
        tile,
        flip_x,
        flip_y,
    })
    .width(24)
    .height(24)
}

pub struct RenderTile {
    tile: Tile,
    flip_x: bool,
    flip_y: bool,
}

impl<Message> canvas::Program<Message> for RenderTile {
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
        frame.scale_nonuniform([
            if self.flip_x { -1.0 } else { 1.0 },
            if self.flip_y { -1.0 } else { 1.0 },
        ]);

        for x in 0..8 {
            for y in 0..8 {
                frame.fill_rectangle(
                    Point::new(
                        x as f32 * widget_size.width / 8.0,
                        y as f32 * widget_size.height / 8.0,
                    ),
                    pixel_size,
                    iced_color(Palette::MONOCHROME_GREEN.color(self.tile.pixel(x, y))),
                );
            }
        }

        vec![frame.into_geometry()]
    }
}
