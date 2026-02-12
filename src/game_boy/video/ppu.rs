use core::fmt;

use crate::game_boy::video::{
    PpuAccessible,
    palette::PaletteIndex,
    screen::{self, Screen},
    sprites::Sprite,
};

use super::{
    sprites::{self, Priority, SpriteSize},
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
    /// Remaining stall dots before the next pixel can be drawn.
    penalty: u32,
    pixels_drawn: u8,
    /// Sprites on this line, sorted by X position (DMG priority).
    sprites: Vec<Sprite>,
    /// Index of the next sprite to check for fetch penalty.
    next_sprite: usize,
    /// Tracks which BG tile columns have already had a sprite penalty
    /// applied, so overlapping sprites in the same tile don't double-count
    /// the alignment cost.
    sprite_tile_penalized: u32,
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
            next_sprite: 0,
            sprite_tile_penalized: 0,
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

        // DMG priority: lower X position wins, ties broken by OAM order (stable sort)
        self.sprites.sort_by_key(|sprite| sprite.position.x_plus_8);

        // SCX penalty: extra dots to discard partial first tile
        let scx = data.background_viewport.x;
        self.penalty += (scx & 7) as u32;
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

    /// Advance by one dot (T-cycle). Returns true when a full frame is complete.
    fn dot(&mut self, data: &PpuAccessible) -> bool {
        if self.line.dots == 0 {
            self.line.find_sprites(data)
        }

        if self.line.dots < SCANLINE_PREPARING_DOTS {
            self.line.dots += 1;
        } else {
            if self.line.pixels_drawn < screen::PIXELS_PER_LINE {
                if self.line.penalty > 0 {
                    self.line.penalty -= 1;
                } else {
                    // Check if a sprite starts at the current pixel X and
                    // inject its fetch penalty before drawing the pixel.
                    self.check_sprite_penalty(data);

                    if self.line.penalty > 0 {
                        self.line.penalty -= 1;
                    } else {
                        self.draw_pixel(data);
                    }
                }
            }

            self.line.dots += 1;

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

        false
    }

    /// Check if any sprites need fetching at the current pixel position.
    /// If so, inject their penalty into the stall counter.
    fn check_sprite_penalty(&mut self, data: &PpuAccessible) {
        let x = self.line.pixels_drawn;
        let scx = data.background_viewport.x;

        while self.line.next_sprite < self.line.sprites.len() {
            let sprite = &self.line.sprites[self.line.next_sprite];

            // Sprite's leftmost visible pixel X = x_plus_8 - 8 (but can be negative/off-screen)
            // The sprite causes a penalty when the PPU reaches the pixel
            // where the sprite starts drawing (or pixel 0 if sprite is partially off-screen left).
            let sprite_screen_x = sprite.position.x_plus_8 as i16 - 8;
            let trigger_x = sprite_screen_x.max(0) as u8;

            if x < trigger_x {
                break; // Haven't reached this sprite yet
            }

            if sprite.position.x_plus_8 == 0 || sprite.position.x_plus_8 >= 168 {
                self.line.next_sprite += 1;
                continue; // Off-screen sprites don't incur penalty
            }

            // Flat 6-dot fetch cost per sprite
            let mut sprite_penalty: u32 = 6;

            // Alignment cost: depends on sprite X relative to background tile grid.
            // If this tile column hasn't been penalized yet, add alignment cost.
            let adjusted_x = sprite.position.x_plus_8.wrapping_add(scx & 7);
            let tile_col = adjusted_x / 8;
            let tile_bit = 1u32 << tile_col;
            if self.line.sprite_tile_penalized & tile_bit == 0 {
                self.line.sprite_tile_penalized |= tile_bit;
                let alignment = 5u32.saturating_sub((adjusted_x & 7) as u32);
                sprite_penalty += alignment;
            }

            self.line.penalty += sprite_penalty;
            self.line.next_sprite += 1;
        }
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
                    let tile_index = if data.control.sprite_size() == SpriteSize::Double {
                        TileIndex(sprite.tile.0 & 0xFE)
                    } else {
                        sprite.tile
                    };
                    let (tile_block_id, tile_id) = TileAddressMode::Block0Block1.tile(tile_index);

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

    /// Returns the OAM row byte offset currently being accessed during
    /// Mode 2, or `None` if the PPU is not scanning OAM.
    ///
    /// During Mode 2, the PPU scans 2 sprites per M-cycle (40 sprites over
    /// 20 M-cycles = 80 dots). The row offset maps the current dot position
    /// to the 8-byte OAM row being accessed.
    pub fn accessed_oam_row(&self) -> Option<u8> {
        match self {
            PixelProcessingUnit::Rendering(rendering) => {
                if rendering.line.dots < SCANLINE_PREPARING_DOTS {
                    Some(((rendering.line.dots / 4 + 1) * 8) as u8)
                } else {
                    None
                }
            }
            PixelProcessingUnit::BetweenFrames(_) => None,
        }
    }

    pub(crate) fn save_state(&self) -> crate::game_boy::save_state::PpuState {
        use crate::game_boy::save_state::{PpuState, ScreenState};

        match self {
            PixelProcessingUnit::Rendering(rendering) => PpuState::Rendering {
                screen: ScreenState::from_screen(&rendering.screen),
                line_number: rendering.line.number,
                line_dots: rendering.line.dots,
                line_penalty: rendering.line.penalty,
                line_pixels_drawn: rendering.line.pixels_drawn,
                line_next_sprite: rendering.line.next_sprite,
                line_sprite_tile_penalized: rendering.line.sprite_tile_penalized,
                line_window_rendered: rendering.line.window_rendered,
                window_line_counter: rendering.window_line_counter,
            },
            PixelProcessingUnit::BetweenFrames(dots) => PpuState::BetweenFrames { dots: *dots },
        }
    }

    pub(crate) fn from_state(state: crate::game_boy::save_state::PpuState) -> Self {
        use crate::game_boy::save_state::PpuState;

        match state {
            PpuState::Rendering {
                screen: screen_state,
                line_number,
                line_dots,
                line_penalty,
                line_pixels_drawn,
                line_next_sprite,
                line_sprite_tile_penalized,
                line_window_rendered,
                window_line_counter,
            } => PixelProcessingUnit::Rendering(Rendering {
                screen: screen_state.to_screen(),
                line: Line {
                    number: line_number,
                    dots: line_dots,
                    penalty: line_penalty,
                    pixels_drawn: line_pixels_drawn,
                    sprites: Vec::new(),
                    next_sprite: line_next_sprite,
                    sprite_tile_penalized: line_sprite_tile_penalized,
                    window_rendered: line_window_rendered,
                },
                window_line_counter,
            }),
            PpuState::BetweenFrames { dots } => PixelProcessingUnit::BetweenFrames(dots),
            PpuState::Off => PixelProcessingUnit::Rendering(Rendering::new()),
        }
    }

    /// Advance the PPU by one dot (T-cycle). Returns a completed screen
    /// when a full frame finishes rendering.
    pub fn tcycle(&mut self, data: &PpuAccessible) -> Option<Screen> {
        let mut screen = None;

        match self {
            PixelProcessingUnit::Rendering(rendering) => {
                if rendering.dot(data) {
                    screen = Some(rendering.screen.clone());
                    *self = PixelProcessingUnit::BetweenFrames(0);
                }
            }
            PixelProcessingUnit::BetweenFrames(dots) => {
                *dots += 1;
                if *dots >= BETWEEN_FRAMES_DOTS {
                    *self = PixelProcessingUnit::Rendering(Rendering::new());
                }
            }
        };

        screen
    }
}
