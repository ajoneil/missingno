//! The raster pipeline: the emitted frame, the cell fetch each segment
//! latches, and the dot the raster puts down.

use crate::Vdp;
use crate::registers::{Mode, pattern_row};
use crate::standard::{ACTIVE_LINES, ACTIVE_WIDTH, LEFT_BORDER, Standard, VISIBLE_WIDTH};

/// Raster placement — the counter-to-picture alignment, the same freedom
/// as the schedule rotation: picture row N emits during counter line N-1,
/// pixel 0 at this XTAL offset. Calibrated against midline-name's seam,
/// whose cell quantisation pins the offset to [24, 40).
const ACTIVE_START_XTALS: u32 = 32;
/// The whole visible span sits inside one counter line, so a counter line
/// carries a complete scanline: left border, display area, right border.
const VISIBLE_START_XTALS: u32 = ACTIVE_START_XTALS - LEFT_BORDER as u32 * 2;
const XTALS_PER_VISIBLE: u32 = VISIBLE_WIDTH as u32 * 2;

/// The 32-cell grid every non-text mode fetches on.
const GRID_COLUMNS: u16 = 32;
const CELL_WIDTH: usize = 8;
/// The text grid: 40 six-pixel cells inside backdrop margins split 6 left
/// / 10 right, the Data Manual's asymmetric split (silicon: modes/text).
const TEXT_COLUMNS: u16 = 40;
const TEXT_CELL_WIDTH: usize = 6;
const TEXT_MARGIN: usize = 6;

/// The canonical datasheet RGB for each of the sixteen colour indices —
/// presentation policy for a chip that emits indices, offered so every
/// consumer stamps the same palette. Index 0 is the all-planes-transparent
/// pass-through and presents as black.
pub const PALETTE: [[u8; 3]; 16] = [
    [0, 0, 0],
    [0, 0, 0],
    [33, 200, 66],
    [94, 220, 120],
    [84, 85, 237],
    [125, 118, 252],
    [212, 82, 77],
    [66, 235, 245],
    [252, 85, 84],
    [255, 121, 120],
    [212, 193, 84],
    [230, 206, 128],
    [33, 176, 59],
    [201, 91, 186],
    [204, 204, 204],
    [255, 255, 255],
];

/// The visible raster — the display area inside its backdrop border — as
/// composited colour indices, row-major. 0 survives only where every plane
/// is transparent (the external-video pass-through) and presents as black.
#[derive(Clone)]
pub struct Frame {
    pub pixels: Vec<u8>,
    pub width: u16,
    pub height: u16,
}

impl Frame {
    pub(crate) fn blank(standard: Standard) -> Self {
        Frame {
            pixels: vec![0; VISIBLE_WIDTH as usize * standard.visible_lines() as usize],
            width: VISIBLE_WIDTH,
            height: standard.visible_lines(),
        }
    }
}

/// One latched fetch: `end_x - start_x` pixels drawn MSB-first from
/// `bits`, lit pixels in `fg`, unlit in `bg`; colour 0 falls through to
/// the live backdrop at emission.
#[derive(Clone, Copy)]
pub(crate) struct Segment {
    pub(crate) bits: u8,
    pub(crate) fg: u8,
    pub(crate) bg: u8,
    pub(crate) start_x: usize,
    pub(crate) end_x: usize,
}

impl Segment {
    pub(crate) const BLANK: Self = Segment {
        bits: 0,
        fg: 0,
        bg: 0,
        start_x: 0,
        end_x: 0,
    };
}

/// Multicolor cells hold four two-pixel rows, each spanning four raster
/// rows, and repeat every four cell rows.
fn multicolor_row(cell_row: u16, row_in_cell: u16) -> u16 {
    (cell_row & 3) * 2 + row_in_cell / 4
}

impl Vdp {
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Count of completed visible rasters — increments as the raster leaves
    /// the bottom border, when every row of `frame` is this frame's.
    pub fn frames_completed(&self) -> u64 {
        self.frames_completed
    }

    pub fn line(&self) -> u16 {
        self.line
    }

    pub fn dot(&self) -> u16 {
        (self.xtal_in_line / 2) as u16
    }

    /// The picture row emitting during the counter line under way.
    pub(crate) fn emitting_row(&self) -> u16 {
        if self.line + 1 == self.standard.lines_per_frame() {
            0
        } else {
            self.line + 1
        }
    }

    /// Emit the dot under the raster: the fetch segment latches from the
    /// live registers at its own instant, transparency resolves against
    /// the live backdrop per dot — mid-line writes land at their
    /// silicon-measured granularities (R7 per pixel, tables and mode bits
    /// per cell; sprites stay line-latched).
    pub(crate) fn raster_dot(&mut self) {
        let Some(offset) = self.xtal_in_line.checked_sub(VISIBLE_START_XTALS) else {
            return;
        };
        if offset >= XTALS_PER_VISIBLE || !offset.is_multiple_of(2) {
            return;
        }
        let visible_x = (offset / 2) as usize;
        let picture_row = self.emitting_row();
        let Some(frame_row) = self.frame_row(picture_row) else {
            return;
        };

        let active_x = (picture_row < ACTIVE_LINES)
            .then(|| visible_x.wrapping_sub(LEFT_BORDER as usize))
            .filter(|&x| x < ACTIVE_WIDTH as usize);
        self.line_pixels[visible_x] = match active_x {
            // The backdrop is the only plane that reaches the border, and
            // no fetch belongs to a border dot.
            None => self.backdrop(),
            Some(x) => {
                if x == 0 || x >= self.segment.end_x {
                    self.segment = self.latch_segment(picture_row, x);
                }
                if !self.display_enabled() {
                    self.backdrop()
                } else if self.sprite_line[x] != 0 {
                    self.sprite_line[x]
                } else {
                    let bit = x - self.segment.start_x;
                    let lit = bit < 8 && self.segment.bits & (0x80 >> bit) != 0;
                    let colour = if lit {
                        self.segment.fg
                    } else {
                        self.segment.bg
                    };
                    self.over_backdrop(colour)
                }
            }
        };
        if visible_x == VISIBLE_WIDTH as usize - 1 {
            let start = frame_row as usize * VISIBLE_WIDTH as usize;
            self.frame.pixels[start..start + VISIBLE_WIDTH as usize]
                .copy_from_slice(&self.line_pixels);
            if frame_row == self.frame.height - 1 {
                self.frames_completed += 1;
            }
        }
    }

    /// Where a picture row lands in the visible raster: the top border rides
    /// the counter wrap, then the display area, then the bottom border;
    /// everything else is blanking.
    fn frame_row(&self, picture_row: u16) -> Option<u16> {
        let top = self.standard.top_border();
        let first_top_row = self.standard.lines_per_frame() - top;
        if picture_row >= first_top_row {
            Some(picture_row - first_top_row)
        } else if picture_row < ACTIVE_LINES + self.standard.bottom_border() {
            // The display area and the bottom border run contiguously.
            Some(top + picture_row)
        } else {
            None
        }
    }

    /// A transparent pixel falls through to the backdrop; a transparent
    /// backdrop stays 0 (the external-video plane, presented black).
    fn over_backdrop(&self, colour: u8) -> u8 {
        if colour != 0 { colour } else { self.backdrop() }
    }

    /// The fetch segment covering pixel `x`, latched from the live
    /// registers and VRAM at this instant — mid-line register writes take
    /// effect from the next segment (silicon: R2 and M2 cell-quantised).
    fn latch_segment(&self, row: u16, x: usize) -> Segment {
        let mode = self.mode();
        if mode.text_grid() {
            self.text_segment(mode, row, x)
        } else {
            self.cell_segment(mode, row, x)
        }
    }

    /// The text grid's fetch: no colour table, R7 carrying both colours,
    /// and backdrop margins either side of the 40 cells.
    fn text_segment(&self, mode: Mode, row: u16, x: usize) -> Segment {
        let left = TEXT_MARGIN;
        let right = TEXT_MARGIN + TEXT_COLUMNS as usize * TEXT_CELL_WIDTH;
        if x < left || x >= right {
            let end_x = if x < left {
                left
            } else {
                ACTIVE_WIDTH as usize
            };
            return Segment {
                start_x: x,
                end_x,
                ..Segment::BLANK
            };
        }
        let col = (x - left) / TEXT_CELL_WIDTH;
        let start_x = left + col * TEXT_CELL_WIDTH;
        // A 0 bit takes R7's low nibble, which is the backdrop register
        // itself — so it rides the live per-dot resolution as bg 0.
        let cell = |bits| Segment {
            bits,
            fg: self.text_colour(),
            bg: 0,
            start_x,
            end_x: start_x + TEXT_CELL_WIDTH,
        };
        if mode == Mode::TextMulticolor {
            // No table reads: four text-colour pixels then two of backdrop.
            return cell(0b1111_0000);
        }
        let table = if mode == Mode::BitmapText {
            self.bitmap_third_table(row / 64)
        } else {
            self.pattern_table_base()
        };
        let name = self.vram_cell(self.name_table_base() + row / 8 * TEXT_COLUMNS + col as u16);
        cell(self.vram_cell(table + pattern_row(name as u16, row % 8)))
    }

    /// The 32-cell grid's fetch: the name selects the pattern, the mode
    /// decides where its two colours come from.
    fn cell_segment(&self, mode: Mode, row: u16, x: usize) -> Segment {
        let cell_row = row / 8;
        let row_in_cell = row % 8;
        let col = (x / CELL_WIDTH) as u16;
        let start_x = col as usize * CELL_WIDTH;
        let name = self.vram_cell(self.name_table_base() + cell_row * GRID_COLUMNS + col) as u16;
        let (bits, fg, bg) = match mode {
            Mode::GraphicsI => {
                let bits =
                    self.vram_cell(self.pattern_table_base() + pattern_row(name, row_in_cell));
                let colours = self.vram_cell(self.colour_table_base() + name / 8);
                (bits, colours >> 4, colours & 0x0F)
            }
            Mode::GraphicsII => {
                let offset = pattern_row(row / 64 * 256 + name, row_in_cell);
                let (bits, colours) = self.graphics_ii_cells(offset);
                (bits, colours >> 4, colours & 0x0F)
            }
            Mode::Multicolor => {
                let byte = self.vram_cell(
                    self.pattern_table_base()
                        + pattern_row(name, multicolor_row(cell_row, row_in_cell)),
                );
                (0b1111_0000, byte >> 4, byte & 0x0F)
            }
            Mode::BitmapMulticolor => {
                // R3's mask governs this fetch too (silicon:
                // undoc-bitmap-multicolor); bitmap text alone is unmasked.
                let table = self.bitmap_third_table(row / 64);
                let offset = pattern_row(name, multicolor_row(cell_row, row_in_cell))
                    & (self.bitmap_mask() & 0x7FF);
                let byte = self.vram_cell(table + offset);
                (0b1111_0000, byte >> 4, byte & 0x0F)
            }
            Mode::Text | Mode::BitmapText | Mode::TextMulticolor => {
                unreachable!("the text family fetches on its own grid")
            }
        };
        Segment {
            bits,
            fg,
            bg,
            start_x,
            end_x: start_x + CELL_WIDTH,
        }
    }
}
