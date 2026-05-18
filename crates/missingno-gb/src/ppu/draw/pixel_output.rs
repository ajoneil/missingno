// Pixel mux: BG/OBJ shift MSBs → priority → palette lookup → LD0/LD1.
// The LCD captures 159 TOBA edges (PX=9..167) plus one post-shift edge at WODU per scanline.

use crate::ppu::{
    PipelineRegisters,
    types::palette::{PaletteIndex, PaletteMap},
};

use super::shifters::{BgShifter, ObjShifter};

/// `sprites_enabled` carries XYLO into the sprite-priority chain (XULA/WOXA → NULY → POKA).
fn resolve_pixel(
    bg_lo: u8,
    bg_hi: u8,
    spr_lo: u8,
    spr_hi: u8,
    spr_pal: u8,
    spr_pri: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,
    bg_window_enabled: bool,
    sprites_enabled: bool,
) -> PaletteIndex {
    let bg_color = if bg_window_enabled {
        (bg_hi << 1) | bg_lo
    } else {
        0
    };

    if sprites_enabled {
        let spr_color = (spr_hi << 1) | spr_lo;
        if spr_color != 0 && (spr_pri == 0 || bg_color == 0) {
            let sprite_palette = if spr_pal == 0 {
                PaletteMap(obp0)
            } else {
                PaletteMap(obp1)
            };
            return sprite_palette.map(PaletteIndex(spr_color));
        }
    }

    PaletteMap(bgp).map(PaletteIndex(bg_color))
}

/// Palettes and LCDC are read live (combinational on hardware). Does not shift the LCD register.
pub(in crate::ppu) fn resolve_current_pixel(
    bg_shifter: &BgShifter,
    obj_shifter: &ObjShifter,
    regs: &PipelineRegisters,
) -> PaletteIndex {
    let bgp = regs.palettes.background_for_bg_resolve();
    let obp0 = regs.palettes.sprite0.output();
    let obp1 = regs.palettes.sprite1.output();
    let bg_window_enabled = regs.bg_window_enabled_for_resolve();
    let sprites_enabled = regs.sprites_enabled_for_resolve();

    let (bg_lo, bg_hi) = bg_shifter.read();
    let (spr_lo, spr_hi, spr_pal, spr_pri) = obj_shifter.read();
    resolve_pixel(
        bg_lo,
        bg_hi,
        spr_lo,
        spr_hi,
        spr_pal,
        spr_pri,
        bgp,
        obp0,
        obp1,
        bg_window_enabled,
        sprites_enabled,
    )
}
