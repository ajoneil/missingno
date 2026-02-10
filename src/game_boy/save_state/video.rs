use nanoserde::{DeRon, SerRon};

use super::base64::Base64Bytes;
use crate::game_boy::video::screen::{self, Screen};

#[derive(SerRon, DeRon)]
pub struct ScreenState {
    pub pixels: Base64Bytes,
}

impl ScreenState {
    pub fn from_screen(screen: &Screen) -> Self {
        let mut pixels =
            Vec::with_capacity(screen::NUM_SCANLINES as usize * screen::PIXELS_PER_LINE as usize);
        for y in 0..screen::NUM_SCANLINES {
            for x in 0..screen::PIXELS_PER_LINE {
                pixels.push(screen.pixel(x, y).0);
            }
        }
        Self {
            pixels: Base64Bytes(pixels),
        }
    }

    pub fn to_screen(&self) -> Screen {
        let mut screen = Screen::new();
        for y in 0..screen::NUM_SCANLINES {
            for x in 0..screen::PIXELS_PER_LINE {
                let idx = y as usize * screen::PIXELS_PER_LINE as usize + x as usize;
                if idx < self.pixels.len() {
                    screen.set_pixel(
                        x,
                        y,
                        crate::game_boy::video::palette::PaletteIndex(self.pixels[idx]),
                    );
                }
            }
        }
        screen
    }
}

#[derive(SerRon, DeRon)]
pub struct VideoState {
    pub control: u8,
    pub background_viewport_x: u8,
    pub background_viewport_y: u8,
    pub window_y: u8,
    pub window_x_plus_7: u8,
    pub background_palette: u8,
    pub sprite0_palette: u8,
    pub sprite1_palette: u8,
    pub interrupt_flags: u8,
    pub current_line_compare: u8,
    pub stat_line_was_high: bool,
    pub tiles: Base64Bytes,
    pub tile_maps: Base64Bytes,
    pub sprites: Base64Bytes,
    pub ppu: PpuState,
}

#[derive(SerRon, DeRon)]
pub enum PpuState {
    Off,
    Rendering {
        screen: ScreenState,
        line_number: u8,
        line_dots: u32,
        line_penalty: u32,
        line_pixels_drawn: u8,
        line_window_rendered: bool,
        window_line_counter: u8,
    },
    BetweenFrames {
        dots: u32,
    },
}
