//! The 315-5124 VDP: timing, ports, and interrupts. Rendering is
//! line-granular — each active line is drawn as it completes, from the
//! register state at that moment. Dot-level write capture within a line
//! is later accuracy work.

pub const DOTS_PER_LINE: u16 = 342;
pub const LINES_PER_FRAME: u16 = 262;
pub const ACTIVE_LINES: u16 = 192;
pub const PIXELS_PER_LINE: usize = 256;

mod status {
    pub const FRAME_INTERRUPT: u8 = 0x80;
    pub const SPRITE_OVERFLOW: u8 = 0x40;
    pub const SPRITE_COLLISION: u8 = 0x20;
}

/// One finished frame: CRAM indices per pixel plus the palette they
/// resolve through, snapshotted as the frame completed.
pub struct Frame {
    pub pixels: Vec<u8>,
    pub cram: [u8; 32],
}

pub struct Vdp {
    pub registers: [u8; 11],
    pub vram: Box<[u8; 0x4000]>,
    pub cram: [u8; 32],

    dot: u16,
    line: u16,

    address: u16,
    code: u8,
    control_latch: Option<u8>,
    read_buffer: u8,
    status: u8,
    line_interrupt_pending: bool,
    line_counter: u8,
    /// R9 latches at frame start; mid-frame writes wait (MacDonald).
    vscroll_latch: u8,

    frame_pixels: Vec<u8>,
    finished_frame: Option<Frame>,
}

impl Default for Vdp {
    fn default() -> Self {
        Self::new()
    }
}

impl Vdp {
    pub fn new() -> Self {
        Vdp {
            registers: [0; 11],
            vram: Box::new([0; 0x4000]),
            cram: [0; 32],
            dot: 0,
            line: 0,
            address: 0,
            code: 0,
            control_latch: None,
            read_buffer: 0,
            status: 0,
            line_interrupt_pending: false,
            line_counter: 0,
            vscroll_latch: 0,
            frame_pixels: vec![0; PIXELS_PER_LINE * ACTIVE_LINES as usize],
            finished_frame: None,
        }
    }

    /// The /INT line as the CPU sees it: level, held until acknowledged
    /// by a status read.
    pub fn interrupt_asserted(&self) -> bool {
        let frame = self.status & status::FRAME_INTERRUPT != 0 && self.registers[1] & 0x20 != 0;
        let line = self.line_interrupt_pending && self.registers[0] & 0x10 != 0;
        frame || line
    }

    pub fn take_frame(&mut self) -> Option<Frame> {
        self.finished_frame.take()
    }

    pub fn line(&self) -> u16 {
        self.line
    }

    pub fn dot(&self) -> u16 {
        self.dot
    }

    /// Advance one dot; lines render and counters advance at line end.
    pub fn step_dot(&mut self) {
        self.dot += 1;
        if self.dot < DOTS_PER_LINE {
            return;
        }
        self.dot = 0;

        if self.line < ACTIVE_LINES {
            self.render_line(self.line);
        }

        // MacDonald's reload rule: the counter decrements on active lines
        // and the line just after; it reloads from R10 everywhere else.
        if self.line <= ACTIVE_LINES {
            if self.line_counter == 0 {
                self.line_counter = self.registers[10];
                self.line_interrupt_pending = true;
            } else {
                self.line_counter -= 1;
            }
        } else {
            self.line_counter = self.registers[10];
        }

        self.line += 1;
        if self.line == ACTIVE_LINES + 1 {
            self.status |= status::FRAME_INTERRUPT;
            self.finished_frame = Some(Frame {
                pixels: std::mem::replace(
                    &mut self.frame_pixels,
                    vec![0; PIXELS_PER_LINE * ACTIVE_LINES as usize],
                ),
                cram: self.cram,
            });
        }
        if self.line == LINES_PER_FRAME {
            self.line = 0;
            self.vscroll_latch = self.registers[9];
        }
    }

    /// Mode 4, line-granular: background tiles, then sprites composed
    /// with the tile priority bit. A disabled display shows the backdrop.
    fn render_line(&mut self, line: u16) {
        let backdrop = 16 + (self.registers[7] & 0x0F);
        let start = line as usize * PIXELS_PER_LINE;
        if self.registers[1] & 0x40 == 0 {
            self.frame_pixels[start..start + PIXELS_PER_LINE].fill(backdrop);
            return;
        }

        let mut background = [0u8; PIXELS_PER_LINE];
        let mut priority = [false; PIXELS_PER_LINE];
        self.render_background(line, &mut background, &mut priority);

        let mut sprites = [0u8; PIXELS_PER_LINE];
        self.render_sprites(line, &mut sprites);

        for x in 0..PIXELS_PER_LINE {
            let bg = background[x];
            let sprite = sprites[x];
            let color = if priority[x] && bg & 0x0F != 0 {
                bg
            } else if sprite & 0x0F != 0 {
                sprite
            } else {
                bg
            };
            self.frame_pixels[start + x] = color;
        }

        // The left-column blank hides scroll pop-in.
        if self.registers[0] & 0x20 != 0 {
            self.frame_pixels[start..start + 8].fill(backdrop);
        }
    }

    fn render_background(
        &self,
        line: u16,
        pixels: &mut [u8; PIXELS_PER_LINE],
        priority: &mut [bool; PIXELS_PER_LINE],
    ) {
        let name_base = ((self.registers[2] & 0x0E) as usize) << 10;
        // The top two tile rows ignore the horizontal scroll when locked.
        let hscroll_locked = self.registers[0] & 0x40 != 0 && line < 16;
        let hscroll = if hscroll_locked { 0 } else { self.registers[8] };

        for x in 0..PIXELS_PER_LINE {
            // The right eight columns can lock vertical scroll.
            let vscroll_locked = self.registers[0] & 0x80 != 0 && x >= 192;
            let vscroll = if vscroll_locked {
                0
            } else {
                self.vscroll_latch
            };
            let row = (line + vscroll as u16) % 224;

            let source_x = (x as u8).wrapping_sub(hscroll) as usize;
            let entry_index = (row as usize >> 3) * 32 + (source_x >> 3);
            let entry_address = name_base + entry_index * 2;
            let entry = u16::from_le_bytes([
                self.vram[entry_address & 0x3FFF],
                self.vram[(entry_address + 1) & 0x3FFF],
            ]);

            let tile = (entry & 0x1FF) as usize;
            let hflip = entry & 0x200 != 0;
            let vflip = entry & 0x400 != 0;
            let palette = ((entry >> 11) & 1) as u8;
            priority[x] = entry & 0x1000 != 0;

            let mut fine_x = (source_x & 7) as u8;
            let mut fine_y = (row & 7) as u8;
            if hflip {
                fine_x = 7 - fine_x;
            }
            if vflip {
                fine_y = 7 - fine_y;
            }
            pixels[x] = palette * 16 + self.tile_pixel(tile, fine_x, fine_y);
        }
    }

    fn render_sprites(&mut self, line: u16, pixels: &mut [u8; PIXELS_PER_LINE]) {
        let sat = ((self.registers[5] & 0x7E) as usize) << 7;
        let height: u16 = if self.registers[1] & 0x02 != 0 { 16 } else { 8 };
        let shift_left = self.registers[0] & 0x08 != 0;
        let pattern_base = ((self.registers[6] & 0x04) as usize) << 6;

        let mut drawn = 0;
        for index in 0..64 {
            let y = self.vram[(sat + index) & 0x3FFF];
            if y == 0xD0 {
                break;
            }
            // Y is an 8-bit counter: $FF puts the sprite's first line
            // at the top of the screen.
            let top = (y as u16 + 1) & 0xFF;
            if line < top || line >= top + height {
                continue;
            }
            drawn += 1;
            if drawn > 8 {
                self.status |= status::SPRITE_OVERFLOW;
                break;
            }

            let x =
                self.vram[(sat + 128 + index * 2) & 0x3FFF] as i16 - if shift_left { 8 } else { 0 };
            let mut tile = self.vram[(sat + 129 + index * 2) & 0x3FFF] as usize + pattern_base;
            let mut row = (line - top) as u8;
            if height == 16 {
                tile &= !1;
                if row >= 8 {
                    tile += 1;
                    row -= 8;
                }
            }

            for pixel in 0..8i16 {
                let screen_x = x + pixel;
                if !(0..PIXELS_PER_LINE as i16).contains(&screen_x) {
                    continue;
                }
                let color = self.tile_pixel(tile, pixel as u8, row);
                if color == 0 {
                    continue;
                }
                let slot = &mut pixels[screen_x as usize];
                if *slot & 0x0F != 0 {
                    self.status |= status::SPRITE_COLLISION;
                } else {
                    // Sprites always use the second palette.
                    *slot = 16 + color;
                }
            }
        }
    }

    /// One pixel of a 4bpp planar tile (4 bitplane bytes per row).
    fn tile_pixel(&self, tile: usize, x: u8, y: u8) -> u8 {
        let row_address = (tile * 32 + y as usize * 4) & 0x3FFF;
        let bit = 7 - x;
        let mut color = 0;
        for plane in 0..4 {
            let byte = self.vram[(row_address + plane) & 0x3FFF];
            color |= ((byte >> bit) & 1) << plane;
        }
        color
    }

    /// The V counter the CPU reads: NTSC 192-line counts $00-$DA then
    /// jumps to $D5-$FF.
    pub fn v_counter(&self) -> u8 {
        if self.line <= 0xDA {
            self.line as u8
        } else {
            (self.line - 6) as u8
        }
    }

    /// The H counter latches on TH transitions (the Light Phaser path);
    /// reads return the latched value. No latch source exists yet, so it
    /// reads the power-on value.
    pub fn h_counter(&self) -> u8 {
        0
    }

    pub fn read_status(&mut self) -> u8 {
        let value = self.status;
        self.status = 0;
        self.line_interrupt_pending = false;
        self.control_latch = None;
        value
    }

    /// Status without the read's acknowledge side effects.
    pub fn peek_status(&self) -> u8 {
        self.status
    }

    pub fn write_control(&mut self, value: u8) {
        match self.control_latch.take() {
            None => self.control_latch = Some(value),
            Some(low) => {
                self.address = u16::from_le_bytes([low, value & 0x3F]);
                self.code = value >> 6;
                if self.code == 0 {
                    self.read_buffer = self.vram[self.address as usize];
                    self.address = (self.address + 1) & 0x3FFF;
                } else if self.code == 2 {
                    let register = (value & 0x0F) as usize;
                    if register < self.registers.len() {
                        self.registers[register] = low;
                    }
                }
            }
        }
    }

    pub fn read_data(&mut self) -> u8 {
        self.control_latch = None;
        let value = self.read_buffer;
        self.read_buffer = self.vram[self.address as usize];
        self.address = (self.address + 1) & 0x3FFF;
        value
    }

    /// Data reads have buffer/address side effects; this has none.
    pub fn peek_data(&self) -> u8 {
        self.read_buffer
    }

    pub fn write_data(&mut self, value: u8) {
        self.control_latch = None;
        self.read_buffer = value;
        if self.code == 3 {
            self.cram[(self.address & 0x1F) as usize] = value;
        } else {
            self.vram[self.address as usize] = value;
        }
        self.address = (self.address + 1) & 0x3FFF;
    }
}
