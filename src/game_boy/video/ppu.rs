use core::fmt;

use crate::game_boy::video::{
    PpuAccessible,
    palette::PaletteIndex,
    screen::{self, Screen},
    sprites::Sprite,
};

use super::{
    sprites::{self, SpriteSize},
    tiles::{TileAddressMode, TileIndex},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

// --- Pixel FIFO types ---

#[derive(Clone, Copy, Default)]
struct FifoPixel {
    /// 2-bit color index (0-3) before palette mapping.
    color: u8,
    /// Sprite palette: 0 = OBP0, 1 = OBP1. Only meaningful in OBJ FIFO.
    palette: u8,
    /// OBJ-to-BG priority bit from sprite attributes.
    bg_priority: bool,
}

/// Fixed-size circular buffer holding up to 8 pixels.
struct PixelFifo {
    pixels: [FifoPixel; 8],
    head: u8,
    len: u8,
}

impl PixelFifo {
    fn new() -> Self {
        Self {
            pixels: [FifoPixel::default(); 8],
            head: 0,
            len: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn len(&self) -> u8 {
        self.len
    }

    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    /// Push 8 pixels. Only valid when FIFO is empty.
    fn push_row(&mut self, pixels: [FifoPixel; 8]) {
        debug_assert!(self.is_empty());
        self.pixels = pixels;
        self.head = 0;
        self.len = 8;
    }

    /// Push a single pixel to the back.
    fn push_one(&mut self, pixel: FifoPixel) {
        debug_assert!(self.len < 8);
        let idx = (self.head + self.len) & 7;
        self.pixels[idx as usize] = pixel;
        self.len += 1;
    }

    /// Pop one pixel from the front.
    fn pop(&mut self) -> FifoPixel {
        debug_assert!(self.len > 0);
        let pixel = self.pixels[self.head as usize];
        self.head = (self.head + 1) & 7;
        self.len -= 1;
        pixel
    }

    /// Get a mutable reference to the pixel at the given offset from head.
    fn get_mut(&mut self, offset: u8) -> &mut FifoPixel {
        let idx = (self.head + offset) & 7;
        &mut self.pixels[idx as usize]
    }

    /// Get the pixel at the given offset from head.
    fn get(&self, offset: u8) -> FifoPixel {
        let idx = (self.head + offset) & 7;
        self.pixels[idx as usize]
    }
}

// --- Background fetcher ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FetcherStep {
    /// Initial penalty dots at scanline start (dummy tile fetch delay).
    Penalty,
    GetTile,
    GetTileDataLow,
    GetTileDataHigh,
    Push,
}

struct Fetcher {
    step: FetcherStep,
    /// Sub-dot counter within the current step (0 or 1 for 2-dot steps).
    dot_in_step: u8,
    /// Tile X coordinate in the tilemap row (increments per tile fetched).
    tile_x: u8,
    /// Cached tile index from GetTile step.
    tile_index: u8,
    /// Cached low byte of tile row from GetTileDataLow step.
    tile_data_low: u8,
    /// Cached high byte of tile row from GetTileDataHigh step.
    tile_data_high: u8,
    /// Whether we're fetching from the window tilemap.
    fetching_window: bool,
}

impl Fetcher {
    fn new() -> Self {
        Self {
            step: FetcherStep::Penalty,
            dot_in_step: 0,
            tile_x: 0,
            tile_index: 0,
            tile_data_low: 0,
            tile_data_high: 0,
            fetching_window: false,
        }
    }

    /// Position within the fetcher cycle.
    /// GetTile: 0-1, DataLow: 2-3, DataHigh: 4-5, Push: 6+
    fn cycle_position(&self) -> u8 {
        match self.step {
            FetcherStep::Penalty => 0,
            FetcherStep::GetTile => self.dot_in_step,
            FetcherStep::GetTileDataLow => 2 + self.dot_in_step,
            FetcherStep::GetTileDataHigh => 4 + self.dot_in_step,
            FetcherStep::Push => 6,
        }
    }
}

// --- Sprite fetch ---

#[derive(Clone, Copy, PartialEq, Eq)]
enum SpriteStep {
    GetTile,
    GetDataLow,
    GetDataHigh,
}

struct SpriteFetch {
    sprite: Sprite,
    step: SpriteStep,
    dot_in_step: u8,
    tile_data_low: u8,
    tile_data_high: u8,
    /// Dots remaining for BG fetcher to reach end of DataHigh before
    /// sprite fetch proper begins (0-5).
    bg_wait_dots: u8,
}

// --- Line state ---

struct Line {
    number: u8,
    dots: u32,
    /// Number of pixels pushed to the LCD (0-160).
    pixels_drawn: u8,
    /// Sprites on this line, sorted by X position (DMG priority).
    sprites: Vec<Sprite>,
    /// Index of the next sprite to check for triggering.
    next_sprite: usize,
    /// Whether the window has been rendered on this line.
    window_rendered: bool,

    /// Background pixel FIFO.
    bg_fifo: PixelFifo,
    /// Object/sprite pixel FIFO (shifts in lockstep with bg_fifo).
    obj_fifo: PixelFifo,
    /// Background/window tile fetcher.
    fetcher: Fetcher,
    /// Pixels to discard from the first BG tile for SCX fine scroll.
    discard_count: u8,
    /// Active sprite fetch, if any.
    sprite_fetch: Option<SpriteFetch>,
}

impl Line {
    fn new(number: u8) -> Self {
        Line {
            number,
            dots: 0,
            pixels_drawn: 0,
            sprites: Vec::new(),
            next_sprite: 0,
            window_rendered: false,
            bg_fifo: PixelFifo::new(),
            obj_fifo: PixelFifo::new(),
            fetcher: Fetcher::new(),
            discard_count: 0,
            sprite_fetch: None,
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

        // SCX fine scroll: discard this many pixels from first tile
        self.discard_count = data.background_viewport.x & 7;
    }
}

// --- Rendering ---

pub struct Rendering {
    screen: Screen,
    line: Line,
    window_line_counter: u8,
    /// After LCD enable, the first line's Mode 2 doesn't begin at dot 0.
    /// The STAT mode bits read as 0 until Mode 2 actually starts.
    lcd_turning_on: bool,
}

impl Rendering {
    fn new() -> Self {
        Rendering {
            screen: Screen::new(),
            line: Line::new(0),
            window_line_counter: 0,
            lcd_turning_on: false,
        }
    }

    fn new_lcd_on() -> Self {
        Rendering {
            screen: Screen::new(),
            line: Line::new(0),
            window_line_counter: 0,
            lcd_turning_on: true,
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

    fn stat_mode(&self) -> Mode {
        // STAT reports mode 0 a couple dots before Mode 3 actually ends.
        // Predict from FIFO state: if no sprites remain and few pixels left.
        if self.mode() == Mode::DrawingPixels
            && self.line.next_sprite >= self.line.sprites.len()
            && self.line.sprite_fetch.is_none()
        {
            let pixels_remaining = (screen::PIXELS_PER_LINE - self.line.pixels_drawn) as u8;
            if pixels_remaining <= 2 && self.line.bg_fifo.len() >= pixels_remaining {
                return Mode::BetweenLines;
            }
        }
        self.mode()
    }

    /// Mode for STAT interrupt edge detection.
    fn interrupt_mode(&self) -> Mode {
        self.mode()
    }

    /// Whether the mode 2 STAT interrupt condition is active.
    fn mode2_interrupt_active(&self) -> bool {
        self.mode() == Mode::PreparingScanline
    }

    fn gating_mode(&self) -> Mode {
        // OAM is locked 4 dots before the scanline ends (at dot 452),
        // before STAT reports mode 2.
        if self.line.dots >= SCANLINE_TOTAL_DOTS - 4 && self.mode() == Mode::BetweenLines {
            Mode::PreparingScanline
        // VRAM is locked 4 dots before mode 3 starts (at dot 76),
        // while STAT still reports mode 2.
        } else if self.line.dots >= SCANLINE_PREPARING_DOTS - 4
            && self.mode() == Mode::PreparingScanline
        {
            Mode::DrawingPixels
        } else {
            self.mode()
        }
    }

    fn write_gating_mode(&self) -> Mode {
        // Writes don't have early locks. Instead, mode 2 releases OAM
        // 4 dots early (at dot 76), creating a brief gap before mode 3.
        if self.line.dots >= SCANLINE_PREPARING_DOTS - 4 && self.mode() == Mode::PreparingScanline {
            Mode::BetweenLines
        } else {
            self.mode()
        }
    }

    /// Advance by one dot (T-cycle). Returns true when a full frame is complete.
    fn dot(&mut self, data: &PpuAccessible) -> bool {
        if self.line.dots == 0 {
            self.line.find_sprites(data);
        }

        if self.line.dots < SCANLINE_PREPARING_DOTS {
            // Mode 2: OAM scan
            self.line.dots += 1;
            if self.line.dots == SCANLINE_PREPARING_DOTS {
                self.lcd_turning_on = false;
            }
        } else {
            // Mode 3 (drawing) and Mode 0 (HBlank)
            if self.line.pixels_drawn < screen::PIXELS_PER_LINE {
                self.dot_mode3(data);
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

    /// One dot of Mode 3 pixel FIFO processing.
    fn dot_mode3(&mut self, data: &PpuAccessible) {
        if let Some(ref mut sf) = self.line.sprite_fetch {
            // Sprite fetch in progress
            if sf.bg_wait_dots > 0 {
                // BG fetcher finishing its cycle before sprite fetch begins
                sf.bg_wait_dots -= 1;
            } else {
                // Advance sprite fetch pipeline
                Self::advance_sprite_fetch(sf, self.line.number, data);
                if sf.step == SpriteStep::GetDataHigh && sf.dot_in_step == 2 {
                    // Sprite fetch just completed — merge and resume
                    Self::merge_sprite_into_obj_fifo(
                        sf,
                        &mut self.line.bg_fifo,
                        &mut self.line.obj_fifo,
                    );
                    self.line.sprite_fetch = None;
                    // Restart BG fetcher
                    self.line.fetcher.step = FetcherStep::GetTile;
                    self.line.fetcher.dot_in_step = 0;
                }
            }
        } else {
            // Normal: advance BG fetcher and try to shift a pixel
            self.advance_bg_fetcher(data);

            if !self.line.bg_fifo.is_empty() {
                // Check for sprite trigger before shifting the pixel out.
                // The trigger must fire when pixels_drawn == sprite X, before
                // that pixel is consumed, so the sprite fetch can populate the
                // OBJ FIFO in time.
                if self.line.pixels_drawn < screen::PIXELS_PER_LINE
                    && self.line.sprite_fetch.is_none()
                {
                    self.check_sprite_trigger(data);
                }

                if self.line.sprite_fetch.is_none() {
                    self.shift_pixel_out(data);
                }
            }
        }
    }

    /// Advance the background tile fetcher by one dot.
    fn advance_bg_fetcher(&mut self, data: &PpuAccessible) {
        let fetcher = &mut self.line.fetcher;

        match fetcher.step {
            FetcherStep::Penalty => {
                if fetcher.dot_in_step >= 5 {
                    fetcher.step = FetcherStep::GetTile;
                    fetcher.dot_in_step = 0;
                } else {
                    fetcher.dot_in_step += 1;
                }
            }
            FetcherStep::GetTile => {
                if fetcher.dot_in_step == 0 {
                    fetcher.dot_in_step = 1;
                } else {
                    // Read tile index from tilemap
                    let (map_x, map_y) = if fetcher.fetching_window {
                        (fetcher.tile_x, self.window_line_counter / 8)
                    } else {
                        let scx = data.background_viewport.x;
                        let scy = data.background_viewport.y;
                        (
                            (fetcher.tile_x.wrapping_add(scx / 8)) & 31,
                            (self.line.number.wrapping_add(scy) / 8) & 31,
                        )
                    };

                    let map_id = if fetcher.fetching_window {
                        data.control.window_tile_map()
                    } else {
                        data.control.background_tile_map()
                    };
                    let map = data.memory.tile_map(map_id);
                    fetcher.tile_index = map.get_tile(map_x, map_y).0;

                    fetcher.dot_in_step = 0;
                    fetcher.step = FetcherStep::GetTileDataLow;
                }
            }
            FetcherStep::GetTileDataLow => {
                if fetcher.dot_in_step == 0 {
                    fetcher.dot_in_step = 1;
                } else {
                    let tile_index = TileIndex(fetcher.tile_index);
                    let (block_id, mapped_idx) = data.control.tile_address_mode().tile(tile_index);

                    let fine_y = if fetcher.fetching_window {
                        self.window_line_counter % 8
                    } else {
                        self.line.number.wrapping_add(data.background_viewport.y) % 8
                    };

                    let block = data.memory.tile_block(block_id);
                    fetcher.tile_data_low =
                        block.data[mapped_idx.0 as usize * 16 + fine_y as usize * 2];

                    fetcher.dot_in_step = 0;
                    fetcher.step = FetcherStep::GetTileDataHigh;
                }
            }
            FetcherStep::GetTileDataHigh => {
                if fetcher.dot_in_step == 0 {
                    fetcher.dot_in_step = 1;
                } else {
                    let tile_index = TileIndex(fetcher.tile_index);
                    let (block_id, mapped_idx) = data.control.tile_address_mode().tile(tile_index);

                    let fine_y = if fetcher.fetching_window {
                        self.window_line_counter % 8
                    } else {
                        self.line.number.wrapping_add(data.background_viewport.y) % 8
                    };

                    let block = data.memory.tile_block(block_id);
                    fetcher.tile_data_high =
                        block.data[mapped_idx.0 as usize * 16 + fine_y as usize * 2 + 1];

                    fetcher.dot_in_step = 0;
                    fetcher.step = FetcherStep::Push;
                }
            }
            FetcherStep::Push => {
                if self.line.bg_fifo.is_empty() {
                    let pixels = decode_tile_row(fetcher.tile_data_low, fetcher.tile_data_high);
                    self.line.bg_fifo.push_row(pixels);
                    fetcher.tile_x = fetcher.tile_x.wrapping_add(1);
                    fetcher.step = FetcherStep::GetTile;
                    fetcher.dot_in_step = 0;
                }
                // If FIFO not empty, stay in Push and retry next dot.
            }
        }
    }

    /// Advance the sprite fetch pipeline by one dot.
    fn advance_sprite_fetch(sf: &mut SpriteFetch, line_number: u8, data: &PpuAccessible) {
        match sf.step {
            SpriteStep::GetTile => {
                if sf.dot_in_step == 0 {
                    sf.dot_in_step = 1;
                } else {
                    // Tile index comes from OAM (already in sprite struct)
                    sf.dot_in_step = 0;
                    sf.step = SpriteStep::GetDataLow;
                }
            }
            SpriteStep::GetDataLow => {
                if sf.dot_in_step == 0 {
                    sf.dot_in_step = 1;
                } else {
                    let sprite = &sf.sprite;
                    let tile_index = if data.control.sprite_size() == SpriteSize::Double {
                        TileIndex(sprite.tile.0 & 0xFE)
                    } else {
                        sprite.tile
                    };
                    let (block_id, mapped_idx) = TileAddressMode::Block0Block1.tile(tile_index);

                    let sprite_y = line_number as i16 + 16 - sprite.position.y_plus_16 as i16;
                    let flipped_y = if sprite.attributes.flip_y() {
                        (data.control.sprite_size().height() as i16 - 1) - sprite_y
                    } else {
                        sprite_y
                    } as u8;

                    let (final_block, final_idx, final_y) = if flipped_y < 8 {
                        (block_id, mapped_idx, flipped_y)
                    } else {
                        (block_id, TileIndex(mapped_idx.0 + 1), flipped_y - 8)
                    };

                    let block = data.memory.tile_block(final_block);
                    sf.tile_data_low = block.data[final_idx.0 as usize * 16 + final_y as usize * 2];

                    sf.dot_in_step = 0;
                    sf.step = SpriteStep::GetDataHigh;
                }
            }
            SpriteStep::GetDataHigh => {
                if sf.dot_in_step == 0 {
                    sf.dot_in_step = 1;
                } else {
                    let sprite = &sf.sprite;
                    let tile_index = if data.control.sprite_size() == SpriteSize::Double {
                        TileIndex(sprite.tile.0 & 0xFE)
                    } else {
                        sprite.tile
                    };
                    let (block_id, mapped_idx) = TileAddressMode::Block0Block1.tile(tile_index);

                    let sprite_y = line_number as i16 + 16 - sprite.position.y_plus_16 as i16;
                    let flipped_y = if sprite.attributes.flip_y() {
                        (data.control.sprite_size().height() as i16 - 1) - sprite_y
                    } else {
                        sprite_y
                    } as u8;

                    let (final_block, final_idx, final_y) = if flipped_y < 8 {
                        (block_id, mapped_idx, flipped_y)
                    } else {
                        (block_id, TileIndex(mapped_idx.0 + 1), flipped_y - 8)
                    };

                    let block = data.memory.tile_block(final_block);
                    sf.tile_data_high =
                        block.data[final_idx.0 as usize * 16 + final_y as usize * 2 + 1];
                    // Signal completion. Use dot_in_step = 2 to distinguish
                    // from the initial entry state (dot_in_step = 0).
                    sf.dot_in_step = 2;
                }
            }
        }
    }

    /// Merge fetched sprite pixels into the OBJ FIFO.
    fn merge_sprite_into_obj_fifo(
        sf: &SpriteFetch,
        bg_fifo: &mut PixelFifo,
        obj_fifo: &mut PixelFifo,
    ) {
        let sprite = &sf.sprite;

        // Decode 8 sprite pixels
        let mut sprite_pixels = [FifoPixel::default(); 8];
        for i in 0..8u8 {
            let bit = if sprite.attributes.flip_x() { i } else { 7 - i };
            let lo = (sf.tile_data_low >> bit) & 1;
            let hi = (sf.tile_data_high >> bit) & 1;
            sprite_pixels[i as usize] = FifoPixel {
                color: (hi << 1) | lo,
                palette: if sprite.attributes.contains(sprites::Attributes::PALETTE) {
                    1
                } else {
                    0
                },
                bg_priority: sprite.attributes.contains(sprites::Attributes::PRIORITY),
            };
        }

        // Sprites partially off-screen left: skip the clipped pixels
        let sprite_screen_x = sprite.position.x_plus_8 as i16 - 8;
        let pixels_clipped_left = if sprite_screen_x < 0 {
            (-sprite_screen_x) as u8
        } else {
            0
        };

        // Pad OBJ FIFO with transparent pixels so all 8 sprite pixels
        // have a slot. Must be at least 8 entries even if BG FIFO is shorter,
        // because the sprite's rightmost pixels may extend beyond the
        // current BG tile fetch.
        let required_len = bg_fifo.len().max(8 - pixels_clipped_left);
        while obj_fifo.len() < required_len {
            obj_fifo.push_one(FifoPixel::default());
        }

        // Overlay sprite pixels — only replace transparent (color 0) slots.
        // DMG priority: sprites are sorted by X, so first sprite wins.
        for i in pixels_clipped_left..8 {
            let fifo_pos = i - pixels_clipped_left;
            if fifo_pos < obj_fifo.len() {
                let existing = obj_fifo.get(fifo_pos);
                if existing.color == 0 {
                    *obj_fifo.get_mut(fifo_pos) = sprite_pixels[i as usize];
                }
            }
        }
    }

    /// Shift one pixel out of the FIFOs and output to the LCD.
    fn shift_pixel_out(&mut self, data: &PpuAccessible) {
        let bg_pixel = self.line.bg_fifo.pop();

        // Pop from OBJ FIFO in lockstep (if it has pixels)
        let obj_pixel = if !self.line.obj_fifo.is_empty() {
            Some(self.line.obj_fifo.pop())
        } else {
            None
        };

        // Discard pixels for SCX fine scroll
        if self.line.discard_count > 0 {
            self.line.discard_count -= 1;
            return;
        }

        if self.line.pixels_drawn >= screen::PIXELS_PER_LINE {
            return;
        }

        let x = self.line.pixels_drawn;
        let y = self.line.number;

        // Background color (0 if BG/window disabled)
        let bg_color = if data.control.background_and_window_enabled() {
            bg_pixel.color
        } else {
            0
        };

        // Check sprite pixel for priority mixing
        if data.control.sprites_enabled() {
            if let Some(sp) = obj_pixel {
                if sp.color != 0 && (!sp.bg_priority || bg_color == 0) {
                    // Sprite pixel wins
                    let sprite_palette = if sp.palette == 0 {
                        &data.palettes.sprite0
                    } else {
                        &data.palettes.sprite1
                    };
                    let mapped = sprite_palette.map(PaletteIndex(sp.color));
                    self.screen.set_pixel(x, y, mapped);
                    self.line.pixels_drawn += 1;
                    self.check_window_trigger(data);
                    return;
                }
            }
        }

        // Background pixel
        let mapped = data.palettes.background.map(PaletteIndex(bg_color));
        self.screen.set_pixel(x, y, mapped);
        self.line.pixels_drawn += 1;
        self.check_window_trigger(data);
    }

    /// Check if the window should start rendering at the current pixel position.
    fn check_window_trigger(&mut self, data: &PpuAccessible) {
        if self.line.fetcher.fetching_window {
            return;
        }
        if !data.control.window_enabled() {
            return;
        }
        if self.line.number < data.window.y {
            return;
        }
        let wx = data.window.x_plus_7.wrapping_sub(7);
        if self.line.pixels_drawn < wx {
            return;
        }

        // Window trigger: clear FIFO and restart fetcher for window tiles
        self.line.bg_fifo.clear();
        self.line.obj_fifo.clear();
        self.line.fetcher.step = FetcherStep::GetTile;
        self.line.fetcher.dot_in_step = 0;
        self.line.fetcher.tile_x = 0;
        self.line.fetcher.fetching_window = true;
        self.line.window_rendered = true;
    }

    /// Check if a sprite should start fetching at the current screen X.
    fn check_sprite_trigger(&mut self, data: &PpuAccessible) {
        if !data.control.sprites_enabled() {
            return;
        }

        let screen_x = self.line.pixels_drawn;

        while self.line.next_sprite < self.line.sprites.len() {
            let sprite = &self.line.sprites[self.line.next_sprite];

            let trigger_x = (sprite.position.x_plus_8 as i16 - 8).max(0) as u8;

            if screen_x < trigger_x {
                break;
            }

            if sprite.position.x_plus_8 >= 168 {
                self.line.next_sprite += 1;
                continue;
            }

            // Compute wait dots: BG fetcher must reach end of DataHigh (position 5)
            let position = self.line.fetcher.cycle_position();
            let bg_wait_dots = if position >= 6 {
                0 // Already past DataHigh (in Push)
            } else {
                5 - position
            };

            self.line.sprite_fetch = Some(SpriteFetch {
                sprite: *sprite,
                step: SpriteStep::GetTile,
                dot_in_step: 0,
                tile_data_low: 0,
                tile_data_high: 0,
                bg_wait_dots,
            });
            self.line.next_sprite += 1;
            break; // Only one sprite fetch at a time
        }
    }
}

/// Decode a tile row (2 bytes) into 8 FIFO pixels.
fn decode_tile_row(low: u8, high: u8) -> [FifoPixel; 8] {
    let mut pixels = [FifoPixel::default(); 8];
    for i in 0..8 {
        let bit = 7 - i;
        let lo = (low >> bit) & 1;
        let hi = (high >> bit) & 1;
        pixels[i] = FifoPixel {
            color: (hi << 1) | lo,
            palette: 0,
            bg_priority: false,
        };
    }
    pixels
}

// --- PixelProcessingUnit enum ---

pub enum PixelProcessingUnit {
    Rendering(Rendering),
    BetweenFrames(u32),
}

impl PixelProcessingUnit {
    pub fn new() -> Self {
        Self::Rendering(Rendering::new())
    }

    /// Create a PPU for an LCD-on transition (LCDC bit 7 set after being
    /// clear). The first line reports mode 0 in STAT until the OAM scan
    /// begins internally.
    pub fn new_lcd_on() -> Self {
        Self::Rendering(Rendering::new_lcd_on())
    }

    pub fn current_line(&self) -> u8 {
        match self {
            PixelProcessingUnit::Rendering(Rendering {
                line: Line { number, dots, .. },
                ..
            }) => {
                if *dots >= SCANLINE_TOTAL_DOTS - 4 {
                    number + 1
                } else {
                    *number
                }
            }
            PixelProcessingUnit::BetweenFrames(dots) => {
                screen::NUM_SCANLINES + (dots / SCANLINE_TOTAL_DOTS) as u8
            }
        }
    }

    /// True on the exact dot where LY increments early (4 dots before
    /// standard scanline end).
    pub fn ly_transitioning(&self) -> bool {
        match self {
            PixelProcessingUnit::Rendering(Rendering {
                line: Line { dots, .. },
                ..
            }) => *dots == SCANLINE_TOTAL_DOTS - 4,
            PixelProcessingUnit::BetweenFrames(dots) => {
                dots % SCANLINE_TOTAL_DOTS == SCANLINE_TOTAL_DOTS - 4
            }
        }
    }

    pub fn mode(&self) -> Mode {
        match self {
            PixelProcessingUnit::Rendering(rendering) => rendering.mode(),
            PixelProcessingUnit::BetweenFrames(_) => Mode::BetweenFrames,
        }
    }

    pub fn stat_mode(&self) -> Mode {
        match self {
            PixelProcessingUnit::Rendering(rendering) if rendering.lcd_turning_on => {
                Mode::BetweenLines
            }
            PixelProcessingUnit::Rendering(rendering) => rendering.stat_mode(),
            PixelProcessingUnit::BetweenFrames(_) => Mode::BetweenFrames,
        }
    }

    pub fn interrupt_mode(&self) -> Mode {
        match self {
            PixelProcessingUnit::Rendering(rendering) if rendering.lcd_turning_on => {
                Mode::BetweenLines
            }
            PixelProcessingUnit::Rendering(rendering) => rendering.interrupt_mode(),
            PixelProcessingUnit::BetweenFrames(_) => Mode::BetweenFrames,
        }
    }

    pub fn mode2_interrupt_active(&self) -> bool {
        match self {
            PixelProcessingUnit::Rendering(rendering) if rendering.lcd_turning_on => false,
            PixelProcessingUnit::Rendering(rendering) => rendering.mode2_interrupt_active(),
            PixelProcessingUnit::BetweenFrames(_) => false,
        }
    }

    pub fn gating_mode(&self) -> Mode {
        match self {
            PixelProcessingUnit::Rendering(rendering) if rendering.lcd_turning_on => {
                Mode::BetweenLines
            }
            PixelProcessingUnit::Rendering(rendering) => rendering.gating_mode(),
            PixelProcessingUnit::BetweenFrames(_) => Mode::BetweenFrames,
        }
    }

    pub fn write_gating_mode(&self) -> Mode {
        match self {
            PixelProcessingUnit::Rendering(rendering) if rendering.lcd_turning_on => {
                Mode::BetweenLines
            }
            PixelProcessingUnit::Rendering(rendering) => rendering.write_gating_mode(),
            PixelProcessingUnit::BetweenFrames(_) => Mode::BetweenFrames,
        }
    }

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
                line_pixels_drawn: rendering.line.pixels_drawn,
                line_next_sprite: rendering.line.next_sprite,
                line_window_rendered: rendering.line.window_rendered,
                window_line_counter: rendering.window_line_counter,
                bg_fifo_pixels: {
                    let mut p = [0u8; 8];
                    for i in 0..rendering.line.bg_fifo.len() {
                        p[i as usize] = rendering.line.bg_fifo.get(i).color;
                    }
                    p
                },
                bg_fifo_head: rendering.line.bg_fifo.head,
                bg_fifo_len: rendering.line.bg_fifo.len,
                fetcher_step: match rendering.line.fetcher.step {
                    FetcherStep::Penalty => 0,
                    FetcherStep::GetTile => 1,
                    FetcherStep::GetTileDataLow => 2,
                    FetcherStep::GetTileDataHigh => 3,
                    FetcherStep::Push => 4,
                },
                fetcher_dot_in_step: rendering.line.fetcher.dot_in_step,
                fetcher_tile_x: rendering.line.fetcher.tile_x,
                discard_count: rendering.line.discard_count,
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
                line_pixels_drawn,
                line_next_sprite,
                line_window_rendered,
                window_line_counter,
                bg_fifo_pixels,
                bg_fifo_head,
                bg_fifo_len,
                fetcher_step,
                fetcher_dot_in_step,
                fetcher_tile_x,
                discard_count,
            } => {
                let mut bg_fifo = PixelFifo::new();
                bg_fifo.head = bg_fifo_head;
                bg_fifo.len = bg_fifo_len;
                for i in 0..bg_fifo_len {
                    let idx = (bg_fifo_head + i) & 7;
                    bg_fifo.pixels[idx as usize].color = bg_fifo_pixels[i as usize];
                }

                let fetcher_step = match fetcher_step {
                    0 => FetcherStep::Penalty,
                    1 => FetcherStep::GetTile,
                    2 => FetcherStep::GetTileDataLow,
                    3 => FetcherStep::GetTileDataHigh,
                    _ => FetcherStep::Push,
                };

                PixelProcessingUnit::Rendering(Rendering {
                    screen: screen_state.to_screen(),
                    line: Line {
                        number: line_number,
                        dots: line_dots,
                        pixels_drawn: line_pixels_drawn,
                        sprites: Vec::new(),
                        next_sprite: line_next_sprite,
                        window_rendered: line_window_rendered,
                        bg_fifo,
                        obj_fifo: PixelFifo::new(),
                        fetcher: Fetcher {
                            step: fetcher_step,
                            dot_in_step: fetcher_dot_in_step,
                            tile_x: fetcher_tile_x,
                            tile_index: 0,
                            tile_data_low: 0,
                            tile_data_high: 0,
                            fetching_window: false,
                        },
                        discard_count,
                        sprite_fetch: None,
                    },
                    window_line_counter,
                    lcd_turning_on: false,
                })
            }
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
