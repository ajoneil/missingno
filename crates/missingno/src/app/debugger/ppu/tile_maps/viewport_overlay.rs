use iced::{
    Rectangle, Renderer, Theme, mouse,
    widget::canvas::{self, Frame, Geometry, Path, Stroke},
};

use crate::app::ui::palette;

/// GB screen width in pixels.
const SCREEN_W: f32 = 160.0;
/// GB screen height in pixels.
const SCREEN_H: f32 = 144.0;
/// GB tile map size in pixels.
const MAP_SIZE: f32 = 256.0;

#[derive(Clone)]
pub struct ViewportOverlay {
    /// Background viewport position (SCX, SCY), if this map is the BG map.
    pub bg_viewport: Option<(u8, u8)>,
    /// Window viewport position (WX, WY), if this map is the window map.
    pub win_viewport: Option<(u8, u8)>,
}

impl<Message> canvas::Program<Message> for ViewportOverlay {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());

        // Derive scale from actual canvas size
        let scale = bounds.width / MAP_SIZE;

        if let Some((scx, scy)) = self.bg_viewport {
            draw_wrapping_rect(
                &mut frame,
                scx as f32 * scale,
                scy as f32 * scale,
                SCREEN_W * scale,
                SCREEN_H * scale,
                MAP_SIZE * scale,
                palette::PURPLE,
                1.5,
            );
        }

        if let Some((wx, wy)) = self.win_viewport {
            let win_w = (160.0 - (wx as f32 - 7.0).max(0.0)).max(0.0);
            let win_h = (144.0 - wy as f32).max(0.0);

            if win_w > 0.0 && win_h > 0.0 {
                draw_rect(
                    &mut frame,
                    0.0,
                    0.0,
                    win_w * scale,
                    win_h * scale,
                    palette::TEAL,
                    1.5,
                );
            }
        }

        vec![frame.into_geometry()]
    }
}

/// Draw a rectangle that wraps around the tile map edges.
fn draw_wrapping_rect(
    frame: &mut Frame,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    map_size: f32,
    color: iced::Color,
    line_width: f32,
) {
    let stroke = Stroke::default().with_color(color).with_width(line_width);

    if x + w <= map_size && y + h <= map_size {
        frame.stroke(
            &Path::rectangle(iced::Point::new(x, y), iced::Size::new(w, h)),
            stroke,
        );
    } else {
        let parts = wrapping_parts(x, y, w, h, map_size);
        for (px, py, pw, ph) in parts {
            frame.stroke(
                &Path::rectangle(iced::Point::new(px, py), iced::Size::new(pw, ph)),
                stroke,
            );
        }
    }
}

/// Split a wrapping rectangle into non-wrapping parts.
fn wrapping_parts(x: f32, y: f32, w: f32, h: f32, map_size: f32) -> Vec<(f32, f32, f32, f32)> {
    let mut parts = Vec::new();

    let wraps_x = x + w > map_size;
    let wraps_y = y + h > map_size;

    let w1 = if wraps_x { map_size - x } else { w };
    let h1 = if wraps_y { map_size - y } else { h };

    // Top-left (always present)
    parts.push((x, y, w1, h1));

    if wraps_x {
        let w2 = w - w1;
        parts.push((0.0, y, w2, h1));

        if wraps_y {
            let h2 = h - h1;
            parts.push((0.0, 0.0, w2, h2));
        }
    }

    if wraps_y {
        let h2 = h - h1;
        parts.push((x, 0.0, w1, h2));
    }

    parts
}

/// Draw a simple rectangle (non-wrapping).
fn draw_rect(
    frame: &mut Frame,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: iced::Color,
    line_width: f32,
) {
    let stroke = Stroke::default().with_color(color).with_width(line_width);

    frame.stroke(
        &Path::rectangle(iced::Point::new(x, y), iced::Size::new(w, h)),
        stroke,
    );
}
