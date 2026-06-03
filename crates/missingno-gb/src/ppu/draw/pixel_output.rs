// Snapshot the BG/OBJ shift MSBs feeding the pixel mux. The model's `resolve`
// turns this into the final framebuffer pixel (LD0/LD1).
// The LCD captures 159 TOBA edges (PX=9..167) plus one post-shift edge at WODU per scanline.

use crate::ppu::model::PixelMux;

use super::shifters::{BgShifter, ObjShifter};

/// BG/OBJ shifter Q outputs on this dot. Palettes/LCDC are read live by `resolve`
/// (combinational on hardware); this does not shift the LCD register.
pub(in crate::ppu) fn current_mux(bg_shifter: &BgShifter, obj_shifter: &ObjShifter) -> PixelMux {
    let (bg_lo, bg_hi) = bg_shifter.read();
    let (spr_lo, spr_hi, spr_pal, spr_pri) = obj_shifter.read();
    PixelMux {
        bg_lo,
        bg_hi,
        spr_lo,
        spr_hi,
        spr_pal,
        spr_pri,
    }
}
