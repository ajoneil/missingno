//! The sprite plane: the per-line effects scan the status flags come from,
//! and the paint that reaches the raster.

use crate::Vdp;
use crate::registers::{pattern_row, r1};
use crate::scan::ScanStop;
use crate::standard::XTALS_PER_TSTATE;

/// Sprite attribute Y value that terminates the scan.
pub const SPRITE_TERMINATOR: u8 = 0xD0;

/// Which walk of the attribute table is running: the effects scan, which
/// latches 5S and the stop and paints nothing, or the paint into the
/// emission plane.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SpritePass {
    Effects,
    Paint,
}

/// What the sprite plane holds for one display column: the frontmost
/// painter's colour, and whether a second sprite pixel landed there.
#[derive(Clone, Copy)]
pub(crate) struct SpritePixel {
    pub(crate) colour: u8,
    pub(crate) coincident: bool,
}

impl SpritePixel {
    pub(crate) const EMPTY: Self = SpritePixel {
        colour: 0,
        coincident: false,
    };
}

/// The size and magnification R1 gives every sprite of a line.
#[derive(Clone, Copy)]
struct SpriteGeometry {
    size16: bool,
    magnified: bool,
    /// Raster rows a sprite covers, magnification included.
    height: u8,
}

impl SpriteGeometry {
    fn of(vdp: &Vdp) -> Self {
        let size16 = vdp.sprites_16x16();
        let magnified = vdp.magnified();
        let pattern_rows = if size16 { 16u8 } else { 8 };
        SpriteGeometry {
            size16,
            magnified,
            height: pattern_rows << (magnified as u8),
        }
    }

    /// The pattern row a sprite at `y` contributes to `line`, magnification
    /// divided out; `None` when the sprite is off that line.
    fn row_on_line(self, y: u8, line: u8) -> Option<u8> {
        let row = line.wrapping_sub(y.wrapping_add(1));
        (row < self.height).then(|| row >> (self.magnified as u8))
    }
}

impl Vdp {
    /// The effects walk: the fifth-sprite latch and the scanner's stop come
    /// from here.
    pub(crate) fn scan_sprites(&mut self) {
        if !self.display_enabled() {
            return;
        }
        self.scanner.stop = self.walk_sprites(self.line as u8, SpritePass::Effects);
    }

    /// Paint `row`'s displayed sprites into the emission plane, marking the
    /// columns a second sprite pixel lands on — the same walk as the
    /// effects scan, latching nothing.
    pub(crate) fn paint_sprites(&mut self, row: u16) {
        if !self.display_enabled() || self.registers[1] & r1::M1 != 0 {
            return;
        }
        self.walk_sprites(row as u8, SpritePass::Paint);
    }

    /// One walk of the attribute table for `line`: the first four sprites on
    /// it reach the plane on a paint pass, and the walk ends where the
    /// scanner does.
    fn walk_sprites(&mut self, line: u8, pass: SpritePass) -> ScanStop {
        let geometry = SpriteGeometry::of(self);
        let attributes = self.sprite_attribute_base();
        let mut occupied = [false; 256];
        let mut matched = 0u8;
        let mut stop = ScanStop::FullWalk;

        for index in 0..32u8 {
            let entry = attributes + index as u16 * 4;
            let y = self.vram_cell(entry);
            if y == SPRITE_TERMINATOR {
                stop = ScanStop::Terminator(index);
                break;
            }
            let Some(row) = geometry.row_on_line(y, line) else {
                continue;
            };
            matched += 1;
            if matched == 5 {
                // The data manual's gate is real: 5S only latches while F
                // is clear, and the first capture holds until a read. The
                // scan itself halts here whatever the flags say. A clearing
                // read in the T-state before the set also wins the race.
                if pass == SpritePass::Effects
                    && !self.status.frame
                    && !self.status.fifth_sprite
                    && self.xtal_total - self.status.last_clear_at > XTALS_PER_TSTATE as u64
                {
                    self.status.fifth_sprite = true;
                    self.status.fifth_sprite_set_at = self.xtal_total;
                    self.status.sprite_field = index;
                }
                stop = ScanStop::FifthMatch(index);
                break;
            }
            if pass == SpritePass::Paint {
                self.render_sprite_row(entry, row, &mut occupied, geometry);
            }
        }

        stop
    }

    fn render_sprite_row(
        &mut self,
        entry: u16,
        row: u8,
        occupied: &mut [bool; 256],
        geometry: SpriteGeometry,
    ) {
        let x = self.vram_cell(entry + 1);
        let name = self.vram_cell(entry + 2);
        let tag = self.vram_cell(entry + 3);
        let early_clock = tag & 0x80 != 0;
        let colour = tag & 0x0F;
        let origin = x as i32 - if early_clock { 32 } else { 0 };

        let pattern = self.sprite_pattern_base();
        let row_bits: u16 = if geometry.size16 {
            // Four consecutive generators: the left half's row, then the
            // right half's sixteen bytes later.
            let base = pattern + pattern_row(name as u16 & 0xFC, row as u16);
            u16::from_be_bytes([self.vram_cell(base), self.vram_cell(base + 16)])
        } else {
            (self.vram_cell(pattern + pattern_row(name as u16, row as u16)) as u16) << 8
        };

        let width = if geometry.size16 { 16 } else { 8 };
        let scale = if geometry.magnified { 2 } else { 1 };
        for bit in 0..width {
            if row_bits & (0x8000 >> bit) == 0 {
                continue;
            }
            for sub in 0..scale {
                let px = origin + bit * scale + sub;
                if !(0..256).contains(&px) {
                    continue;
                }
                let pixel = &mut self.sprite_line[px as usize];
                // Coincidence counts every sprite pixel, transparent colour
                // included, and is not gated by F.
                pixel.coincident |= occupied[px as usize];
                occupied[px as usize] = true;
                // A transparent sprite pixel collides but masks nothing;
                // among painters the frontmost wins.
                if colour != 0 && pixel.colour == 0 {
                    pixel.colour = colour;
                }
            }
        }
    }
}
