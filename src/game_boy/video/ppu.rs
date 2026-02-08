use core::fmt;
use std::cmp::min;

use crate::game_boy::video::{
    PpuAccessible,
    palette::PaletteIndex,
    screen::{self, Screen},
    sprites::Sprite,
};

use super::{
    sprites::{self, Priority},
    tiles::{TileAddressMode, TileIndex},
};

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    BetweenFrames = 1,
    PreparingScanline = 2,
    DrawingPixels = 3,
    BetweenLines = 0,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::BetweenFrames => write!(f, "Between Frames"),
            Mode::PreparingScanline => write!(f, "Preparing Scanline"),
            Mode::DrawingPixels => write!(f, "Drawing Pixels"),
            Mode::BetweenLines => write!(f, "Between Scanlines"),
        }
    }
}

const SCANLINE_TOTAL_DOTS: u32 = 456;
const SCANLINE_PREPARING_DOTS: u32 = 80;
const BETWEEN_FRAMES_DOTS: u32 = SCANLINE_TOTAL_DOTS * 10;
const MAX_SPRITES_PER_LINE: usize = 10;

pub struct Rendering {
    screen: Screen,
    line: Line,
    window_line_counter: u8,
}

struct Line {
    number: u8,
    dots: u32,
    penalty: u32,
    pixels_drawn: u8,
    sprites: Vec<Sprite>,
    window_rendered: bool,
}

impl Line {
    fn new(number: u8) -> Self {
        Line {
            number,
            dots: 0,
            penalty: 12,
            pixels_drawn: 0,
            sprites: Vec::new(),
            window_rendered: false,
        }
    }

    fn find_sprites(&mut self, data: &PpuAccessible) {
        self.sprites = data
            .memory
            .sprites()
            .iter()
            .filter(|sprite| {
                sprite
                    .position
                    .on_line(self.number, data.control.sprite_size())
            })
            .take(MAX_SPRITES_PER_LINE)
            .cloned()
            .collect();
    }
}

impl Rendering {
    fn new() -> Self {
        Rendering {
            screen: Screen::new(),
            line: Line::new(0),
            window_line_counter: 0,
        }
    }

    fn mode(&self) -> Mode {
        if self.line.dots < SCANLINE_PREPARING_DOTS {
            Mode::PreparingScanline
        } else if self.line.pixels_drawn < screen::PIXELS_PER_LINE {
            Mode::DrawingPixels
        } else {
            Mode::BetweenLines
        }
    }

    fn render(&mut self, data: &PpuAccessible) -> bool {
        let mut remaining_dots = 4;

        for _ in 0..4 {
            if self.line.dots == 0 {
                self.line.find_sprites(data)
            }

            if self.line.dots < SCANLINE_PREPARING_DOTS {
                let time_preparing = min(remaining_dots, SCANLINE_PREPARING_DOTS - self.line.dots);
                self.line.dots += time_preparing;
                remaining_dots -= time_preparing;
            } else {
                while self.line.pixels_drawn < screen::PIXELS_PER_LINE && remaining_dots > 0 {
                    if self.line.penalty > 0 {
                        self.line.penalty -= 1;
                    } else {
                        self.draw_pixel(data);
                    }

                    self.line.dots += 1;
                    remaining_dots -= 1;
                }

                let time_waiting = min(remaining_dots, SCANLINE_TOTAL_DOTS - self.line.dots);
                if time_waiting > 0 {
                    self.line.dots += time_waiting;
                    remaining_dots -= time_waiting;
                }

                if self.line.dots == SCANLINE_TOTAL_DOTS {
                    if self.line.window_rendered {
                        self.window_line_counter += 1;
                    }
                    self.line = Line::new(self.line.number + 1);

                    if self.line.number == screen::NUM_SCANLINES {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn draw_pixel(&mut self, data: &PpuAccessible) {
        let x = self.line.pixels_drawn;
        let y = self.line.number;

        let mut pixel = PaletteIndex(0);

        if data.control.background_and_window_enabled() {
            let window_x = data.window.x_plus_7.wrapping_sub(7);
            let in_window = data.control.window_enabled() && y >= data.window.y && x >= window_x;

            if in_window {
                let map = data.memory.tile_map(data.control.window_tile_map());
                let map_x = x - window_x;
                let map_y = self.window_line_counter;
                let tile_index = map.get_tile(map_x / 8, map_y / 8);

                let (tile_block, mapped_index) = data.control.tile_address_mode().tile(tile_index);

                let tile = data.memory.tile_block(tile_block).tile(mapped_index);
                pixel = tile.pixel(map_x % 8, map_y % 8);
                self.line.window_rendered = true;
            } else {
                let map = data.memory.tile_map(data.control.background_tile_map());
                let map_x = x.wrapping_add(data.background_viewport.x);
                let map_y = y.wrapping_add(data.background_viewport.y);
                let tile_index = map.get_tile(map_x / 8, map_y / 8);

                let (tile_block, mapped_index) = data.control.tile_address_mode().tile(tile_index);

                let tile = data.memory.tile_block(tile_block).tile(mapped_index);
                pixel = tile.pixel(map_x % 8, map_y % 8);
            }
        };

        if data.control.sprites_enabled() {
            let sprite_pixel = self.line.sprites.iter().find_map(|sprite| {
                let sprite_x = x as i16 + 8 - sprite.position.x_plus_8 as i16;
                if (0..8).contains(&sprite_x) {
                    let (tile_block_id, tile_id) = TileAddressMode::Block0Block1.tile(sprite.tile);

                    let flipped_x = if sprite.attributes.flip_x() {
                        7 - sprite_x as u8
                    } else {
                        sprite_x as u8
                    };
                    let sprite_y = y + 16 - sprite.position.y_plus_16;
                    let flipped_y = if sprite.attributes.flip_y() {
                        (data.control.sprite_size().height() - 1) - sprite_y as u8
                    } else {
                        sprite_y as u8
                    };

                    let sprite_pixel = if flipped_y < 8 {
                        data.memory
                            .tile_block(tile_block_id)
                            .tile(tile_id)
                            .pixel(flipped_x, flipped_y)
                    } else {
                        data.memory
                            .tile_block(tile_block_id)
                            .tile(TileIndex(tile_id.0 + 1))
                            .pixel(flipped_x, flipped_y - 8)
                    };
                    if sprite_pixel.0 > 0 {
                        Some((
                            sprite_pixel,
                            sprite.attributes.priority(),
                            sprite.attributes.palette(),
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

            if let Some((sprite_pixel, priority, palette)) = sprite_pixel {
                if priority == Priority::Sprite || pixel.0 == 0 {
                    let sprite_palette = match palette {
                        sprites::Palette::Palette0 => &data.palettes.sprite0,
                        sprites::Palette::Palette1 => &data.palettes.sprite1,
                    };
                    pixel = sprite_palette.map(sprite_pixel);
                    self.screen.set_pixel(x, y, pixel);
                    self.line.pixels_drawn += 1;
                    return;
                }
            }
        }

        pixel = data.palettes.background.map(pixel);
        self.screen.set_pixel(x, y, pixel);

        self.line.pixels_drawn += 1;
    }
}

pub enum PixelProcessingUnit {
    Rendering(Rendering),
    BetweenFrames(u32),
}

impl PixelProcessingUnit {
    pub fn new() -> Self {
        Self::Rendering(Rendering::new())
    }

    pub fn current_line(&self) -> u8 {
        match self {
            PixelProcessingUnit::Rendering(Rendering {
                line: Line { number, .. },
                ..
            }) => *number,
            PixelProcessingUnit::BetweenFrames(dots) => {
                screen::NUM_SCANLINES + (dots / SCANLINE_TOTAL_DOTS) as u8
            }
        }
    }

    pub fn mode(&self) -> Mode {
        match self {
            PixelProcessingUnit::Rendering(rendering) => rendering.mode(),
            PixelProcessingUnit::BetweenFrames(_) => Mode::BetweenFrames,
        }
    }

    pub fn tick(&mut self, data: &PpuAccessible) -> Option<Screen> {
        let mut screen = None;

        match self {
            PixelProcessingUnit::Rendering(rendering) => {
                if rendering.render(data) {
                    screen = Some(rendering.screen.clone());
                    *self = PixelProcessingUnit::BetweenFrames(0);
                }
            }
            PixelProcessingUnit::BetweenFrames(dots) => {
                *dots += 4;
                if *dots >= BETWEEN_FRAMES_DOTS {
                    *self = PixelProcessingUnit::Rendering(Rendering::new());
                }
            }
        };

        screen
    }
}
