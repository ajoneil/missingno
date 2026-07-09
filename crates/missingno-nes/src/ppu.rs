//! The 2C02: register file, frame timing, and a scanline renderer.
//!
//! Scroll state is the hardware's loopy v/t/x/w set, updated by the
//! register writes exactly as wired; rendering is line-granular from that
//! state, with the horizontal bits recopied and fine Y incremented at
//! line boundaries rather than at their exact dots — the dot-exact fetch
//! pipeline is later accuracy work. Sprite-zero hits latch at their
//! computed pixel as the dot counter passes it.

use crate::cartridge::{Cartridge, Mirroring};

pub const DOTS_PER_LINE: u16 = 341;
pub const LINES_PER_FRAME: u16 = 262;
pub const VISIBLE_LINES: u16 = 240;
pub const PIXELS_PER_LINE: usize = 256;
const VBLANK_LINE: u16 = 241;
const PRERENDER_LINE: u16 = 261;

/// One finished frame of 6-bit NES colour values (palette-RAM resolved).
pub struct Frame {
    pub pixels: Vec<u8>,
}

pub struct Ppu {
    pub control: u8,
    pub mask: u8,
    status: u8,
    oam_address: u8,
    pub oam: [u8; 256],
    palette: [u8; 32],
    nametables: [u8; 0x800],

    // The loopy scroll registers.
    v: u16,
    t: u16,
    fine_x: u8,
    write_latch: bool,
    read_buffer: u8,

    dot: u16,
    line: u16,
    odd_frame: bool,
    nmi_edge: bool,
    sprite_zero_hit_at: Option<u16>,

    frame_pixels: Vec<u8>,
    finished_frame: Option<Frame>,
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

impl Ppu {
    pub fn new() -> Self {
        Ppu {
            control: 0,
            mask: 0,
            status: 0,
            oam_address: 0,
            oam: [0; 256],
            palette: [0; 32],
            nametables: [0; 0x800],
            v: 0,
            t: 0,
            fine_x: 0,
            write_latch: false,
            read_buffer: 0,
            dot: 0,
            line: 0,
            odd_frame: false,
            nmi_edge: false,
            sprite_zero_hit_at: None,
            frame_pixels: vec![0; PIXELS_PER_LINE * VISIBLE_LINES as usize],
            finished_frame: None,
        }
    }

    pub fn line(&self) -> u16 {
        self.line
    }

    pub fn dot(&self) -> u16 {
        self.dot
    }

    pub fn scroll_state(&self) -> (u16, u16, u8) {
        (self.v, self.t, self.fine_x)
    }

    pub fn take_frame(&mut self) -> Option<Frame> {
        self.finished_frame.take()
    }

    /// The NMI line pulsed this dot (PPUCTRL-gated VBL start).
    pub fn take_nmi(&mut self) -> bool {
        std::mem::take(&mut self.nmi_edge)
    }

    fn rendering_enabled(&self) -> bool {
        self.mask & 0x18 != 0
    }

    pub fn step_dot(&mut self, cartridge: &Cartridge) {
        if self.line < VISIBLE_LINES {
            if self.dot == 0 {
                self.render_line(cartridge);
            }
            if let Some(hit_dot) = self.sprite_zero_hit_at
                && self.dot == hit_dot
            {
                self.status |= 0x40;
                self.sprite_zero_hit_at = None;
            }
            if self.dot == 256 && self.rendering_enabled() {
                self.increment_fine_y();
            }
            if self.dot == 257 && self.rendering_enabled() {
                // Recopy the horizontal scroll bits from t.
                self.v = (self.v & !0x041F) | (self.t & 0x041F);
            }
        }

        if self.line == VBLANK_LINE && self.dot == 1 {
            self.status |= 0x80;
            if self.control & 0x80 != 0 {
                self.nmi_edge = true;
            }
            self.finished_frame = Some(Frame {
                pixels: std::mem::replace(
                    &mut self.frame_pixels,
                    vec![0; PIXELS_PER_LINE * VISIBLE_LINES as usize],
                ),
            });
        }
        if self.line == PRERENDER_LINE {
            if self.dot == 1 {
                self.status &= !0xE0;
                self.sprite_zero_hit_at = None;
            }
            if (280..=304).contains(&self.dot) && self.rendering_enabled() {
                // Recopy the vertical scroll bits from t.
                self.v = (self.v & !0x7BE0) | (self.t & 0x7BE0);
            }
        }

        // Odd rendering frames drop the pre-render line's last idle dot.
        let line_length =
            if self.line == PRERENDER_LINE && self.odd_frame && self.rendering_enabled() {
                DOTS_PER_LINE - 1
            } else {
                DOTS_PER_LINE
            };
        self.dot += 1;
        if self.dot == line_length {
            self.dot = 0;
            self.line += 1;
            if self.line == LINES_PER_FRAME {
                self.line = 0;
                self.odd_frame = !self.odd_frame;
            }
        }
    }

    fn increment_fine_y(&mut self) {
        if self.v & 0x7000 != 0x7000 {
            self.v += 0x1000;
        } else {
            self.v &= !0x7000;
            let mut coarse_y = (self.v >> 5) & 0x1F;
            if coarse_y == 29 {
                coarse_y = 0;
                self.v ^= 0x0800;
            } else if coarse_y == 31 {
                coarse_y = 0;
            } else {
                coarse_y += 1;
            }
            self.v = (self.v & !0x03E0) | (coarse_y << 5);
        }
    }

    fn render_line(&mut self, cartridge: &Cartridge) {
        let line = self.line;
        let start = line as usize * PIXELS_PER_LINE;
        let backdrop = self.palette[0] & 0x3F;
        if !self.rendering_enabled() {
            self.frame_pixels[start..start + PIXELS_PER_LINE].fill(backdrop);
            return;
        }

        let mut background = [0u8; PIXELS_PER_LINE];
        if self.mask & 0x08 != 0 {
            self.render_background(cartridge, &mut background);
        }
        let mut sprites = [(0u8, false, false); PIXELS_PER_LINE];
        if self.mask & 0x10 != 0 {
            self.render_sprites(cartridge, line, &mut sprites);
        }

        for x in 0..PIXELS_PER_LINE {
            let bg = if self.mask & 0x02 == 0 && x < 8 {
                0
            } else {
                background[x]
            };
            let (sprite, behind, is_zero) = if self.mask & 0x04 == 0 && x < 8 {
                (0, false, false)
            } else {
                sprites[x]
            };

            if is_zero && bg & 0x03 != 0 && sprite & 0x03 != 0 && x < 255 {
                self.sprite_zero_hit_at.get_or_insert(x as u16 + 1);
            }

            let palette_index = if sprite & 0x03 != 0 && (bg & 0x03 == 0 || !behind) {
                0x10 + sprite
            } else if bg & 0x03 != 0 {
                bg
            } else {
                0
            };
            self.frame_pixels[start + x] = self.palette_entry(palette_index) & 0x3F;
        }
    }

    fn render_background(&self, cartridge: &Cartridge, pixels: &mut [u8; PIXELS_PER_LINE]) {
        let pattern_base: u16 = if self.control & 0x10 != 0 { 0x1000 } else { 0 };
        let fine_y = (self.v >> 12) & 0x7;
        let mut v = self.v;
        let mut fine_x = self.fine_x;

        for pixel in pixels.iter_mut() {
            let tile_address = 0x2000 | (v & 0x0FFF);
            let tile = self.read_nametable(tile_address, cartridge.mirroring) as u16;
            let attribute_address = 0x23C0 | (v & 0x0C00) | ((v >> 4) & 0x38) | ((v >> 2) & 0x07);
            let attribute = self.read_nametable(attribute_address, cartridge.mirroring);
            let shift = ((v >> 4) & 4) | (v & 2);
            let palette = (attribute >> shift) & 0x03;

            let row = pattern_base + tile * 16 + fine_y;
            let low = cartridge.read_chr(row);
            let high = cartridge.read_chr(row + 8);
            let bit = 7 - fine_x;
            let color = ((low >> bit) & 1) | (((high >> bit) & 1) << 1);

            *pixel = if color == 0 { 0 } else { palette * 4 + color };

            fine_x += 1;
            if fine_x == 8 {
                fine_x = 0;
                // Coarse X increment with nametable wrap.
                if v & 0x001F == 31 {
                    v &= !0x001F;
                    v ^= 0x0400;
                } else {
                    v += 1;
                }
            }
        }
    }

    fn render_sprites(
        &mut self,
        cartridge: &Cartridge,
        line: u16,
        pixels: &mut [(u8, bool, bool); PIXELS_PER_LINE],
    ) {
        let tall = self.control & 0x20 != 0;
        let height = if tall { 16 } else { 8 };
        let mut on_line = 0;

        for index in 0..64 {
            let base = index * 4;
            let top = self.oam[base] as u16 + 1;
            if line < top || line >= top + height {
                continue;
            }
            on_line += 1;
            if on_line > 8 {
                self.status |= 0x20;
                break;
            }

            let tile = self.oam[base + 1];
            let attributes = self.oam[base + 2];
            let x = self.oam[base + 3] as usize;
            let palette = attributes & 0x03;
            let behind = attributes & 0x20 != 0;
            let flip_h = attributes & 0x40 != 0;
            let flip_v = attributes & 0x80 != 0;

            let mut row = (line - top) as u8;
            if flip_v {
                row = (height - 1) as u8 - row;
            }
            let pattern = if tall {
                let bank = ((tile & 1) as u16) << 12;
                let mut tile = (tile & !1) as u16;
                if row >= 8 {
                    tile += 1;
                    row -= 8;
                }
                bank + tile * 16
            } else {
                let bank: u16 = if self.control & 0x08 != 0 { 0x1000 } else { 0 };
                bank + tile as u16 * 16
            };
            let low = cartridge.read_chr(pattern + row as u16);
            let high = cartridge.read_chr(pattern + row as u16 + 8);

            for column in 0..8usize {
                let screen_x = x + column;
                if screen_x >= PIXELS_PER_LINE {
                    break;
                }
                let bit = if flip_h { column } else { 7 - column };
                let color = ((low >> bit) & 1) | (((high >> bit) & 1) << 1);
                if color == 0 || pixels[screen_x].0 & 0x03 != 0 {
                    continue;
                }
                pixels[screen_x] = (palette * 4 + color, behind, index == 0);
            }
        }
    }

    fn nametable_offset(address: u16, mirroring: Mirroring) -> usize {
        let address = (address as usize - 0x2000) & 0x0FFF;
        let table = address / 0x400;
        let offset = address & 0x3FF;
        let physical = match mirroring {
            Mirroring::Vertical => table & 1,
            Mirroring::Horizontal => table >> 1,
        };
        physical * 0x400 + offset
    }

    fn read_nametable(&self, address: u16, mirroring: Mirroring) -> u8 {
        self.nametables[Self::nametable_offset(address, mirroring)]
    }

    /// Palette RAM with the sprite-backdrop mirrors ($3F10/14/18/1C).
    fn palette_entry(&self, index: u8) -> u8 {
        let index = (index & 0x1F) as usize;
        let index = if index >= 0x10 && index.is_multiple_of(4) {
            index - 0x10
        } else {
            index
        };
        self.palette[index]
    }

    // --- CPU-visible registers ($2000-$2007) -----------------------------

    pub fn write_register(&mut self, register: u16, value: u8, cartridge: &mut Cartridge) {
        match register & 7 {
            0 => {
                let was_enabled = self.control & 0x80 != 0;
                self.control = value;
                self.t = (self.t & !0x0C00) | (((value & 0x03) as u16) << 10);
                // Enabling NMI during vblank raises it immediately.
                if !was_enabled && value & 0x80 != 0 && self.status & 0x80 != 0 {
                    self.nmi_edge = true;
                }
            }
            1 => self.mask = value,
            3 => self.oam_address = value,
            4 => {
                self.oam[self.oam_address as usize] = value;
                self.oam_address = self.oam_address.wrapping_add(1);
            }
            5 => {
                if self.write_latch {
                    self.t = (self.t & !0x73E0)
                        | (((value & 0xF8) as u16) << 2)
                        | (((value & 0x07) as u16) << 12);
                } else {
                    self.t = (self.t & !0x001F) | ((value >> 3) as u16);
                    self.fine_x = value & 0x07;
                }
                self.write_latch = !self.write_latch;
            }
            6 => {
                if self.write_latch {
                    self.t = (self.t & 0xFF00) | value as u16;
                    self.v = self.t;
                } else {
                    self.t = (self.t & 0x00FF) | (((value & 0x3F) as u16) << 8);
                }
                self.write_latch = !self.write_latch;
            }
            7 => {
                self.write_memory(self.v & 0x3FFF, value, cartridge);
                self.v = self.v.wrapping_add(self.address_increment()) & 0x7FFF;
            }
            _ => {}
        }
    }

    pub fn read_register(&mut self, register: u16, cartridge: &Cartridge) -> u8 {
        match register & 7 {
            2 => {
                let value = self.status;
                self.status &= !0x80;
                self.write_latch = false;
                value
            }
            4 => self.oam[self.oam_address as usize],
            7 => {
                let address = self.v & 0x3FFF;
                let value = if address >= 0x3F00 {
                    // Palette reads bypass the buffer, which still refills
                    // from the nametable underneath.
                    self.read_buffer =
                        self.read_nametable(0x2000 | (address & 0x0FFF), cartridge.mirroring);
                    self.palette_entry(address as u8)
                } else {
                    let value = self.read_buffer;
                    self.read_buffer = self.read_memory(address, cartridge);
                    value
                };
                self.v = self.v.wrapping_add(self.address_increment()) & 0x7FFF;
                value
            }
            _ => 0,
        }
    }

    /// Register state without read side effects, for inspection.
    pub fn peek_status(&self) -> u8 {
        self.status
    }

    fn address_increment(&self) -> u16 {
        if self.control & 0x04 != 0 { 32 } else { 1 }
    }

    fn read_memory(&self, address: u16, cartridge: &Cartridge) -> u8 {
        match address {
            0x0000..=0x1FFF => cartridge.read_chr(address),
            0x2000..=0x3EFF => {
                self.read_nametable(0x2000 | (address & 0x0FFF), cartridge.mirroring)
            }
            _ => self.palette_entry(address as u8),
        }
    }

    fn write_memory(&mut self, address: u16, value: u8, cartridge: &mut Cartridge) {
        match address {
            0x0000..=0x1FFF => cartridge.write_chr(address, value),
            0x2000..=0x3EFF => {
                let offset =
                    Self::nametable_offset(0x2000 | (address & 0x0FFF), cartridge.mirroring);
                self.nametables[offset] = value;
            }
            _ => {
                let index = (address as usize) & 0x1F;
                let index = if index >= 0x10 && index.is_multiple_of(4) {
                    index - 0x10
                } else {
                    index
                };
                self.palette[index] = value;
            }
        }
    }
}

/// The canonical 2C02 master palette (64 entries), approximated from the
/// standard NTSC values — a display-side stage, not a hardware claim.
/// Frame pixels index into this table.
pub fn master_palette() -> &'static [(u8, u8, u8); 64] {
    const RGB: [(u8, u8, u8); 64] = [
        (84, 84, 84),
        (0, 30, 116),
        (8, 16, 144),
        (48, 0, 136),
        (68, 0, 100),
        (92, 0, 48),
        (84, 4, 0),
        (60, 24, 0),
        (32, 42, 0),
        (8, 58, 0),
        (0, 64, 0),
        (0, 60, 0),
        (0, 50, 60),
        (0, 0, 0),
        (0, 0, 0),
        (0, 0, 0),
        (152, 150, 152),
        (8, 76, 196),
        (48, 50, 236),
        (92, 30, 228),
        (136, 20, 176),
        (160, 20, 100),
        (152, 34, 32),
        (120, 60, 0),
        (84, 90, 0),
        (40, 114, 0),
        (8, 124, 0),
        (0, 118, 40),
        (0, 102, 120),
        (0, 0, 0),
        (0, 0, 0),
        (0, 0, 0),
        (236, 238, 236),
        (76, 154, 236),
        (120, 124, 236),
        (176, 98, 236),
        (228, 84, 236),
        (236, 88, 180),
        (236, 106, 100),
        (212, 136, 32),
        (160, 170, 0),
        (116, 196, 0),
        (76, 208, 32),
        (56, 204, 108),
        (56, 180, 204),
        (60, 60, 60),
        (0, 0, 0),
        (0, 0, 0),
        (236, 238, 236),
        (168, 204, 236),
        (188, 188, 236),
        (212, 178, 236),
        (236, 174, 236),
        (236, 174, 212),
        (236, 180, 176),
        (228, 196, 144),
        (204, 210, 120),
        (180, 222, 120),
        (168, 226, 144),
        (152, 226, 180),
        (160, 214, 228),
        (160, 162, 160),
        (0, 0, 0),
        (0, 0, 0),
    ];
    &RGB
}
