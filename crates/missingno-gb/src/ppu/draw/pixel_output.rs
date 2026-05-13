// Pixel mux: combines BG and OBJ shift register outputs into a single
// colour index, applies sprite priority, and maps through the active
// palette. Hardware chain: BG plane gates via LCDC.0 (VYXE) at RAJY /
// TADE; sprite priority MUX via RYFU / XULA / WOXA / NULY; palette
// lookup via AO2222 combiners MOKA / NURA / WUFU combined in PATY
// (OR3); final pad drivers RAVO (LD1) and REMY (LD0). The LCD glass
// captures LD0/LD1 at each cp_pad rising edge — 159 TOBA-driven
// edges during PX=9..167 plus one extra "post-shift" edge at WODU
// emit the 160 visible pixels per scanline.

use crate::ppu::{
    PipelineRegisters,
    types::palette::{PaletteIndex, PaletteMap},
};

use super::shifters::{BgShifter, ObjShifter};

/// Pixel-MUX sprite-output chain: `xula` (plane-B AND2, xylo & wufy),
/// `woxa` (plane-A AND2, xylo & vupy), `nuly` NOR2, `poka` NOR3.
/// `sprites_enabled` carries XYLO.
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
    // Form 2-bit BG color index (0 if BG/window disabled via LCDC.0)
    let bg_color = if bg_window_enabled {
        (bg_hi << 1) | bg_lo
    } else {
        0
    };

    // Sprite priority mixing
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

    // Background pixel
    PaletteMap(bgp).map(PaletteIndex(bg_color))
}

/// Resolve the current pipe MSBs into a palette index for the LCD
/// data latch (REMY/RAVO). Reads palette and LCDC values live from
/// registers — on hardware, the palette lookup is combinational with
/// no pipeline delay.
///
/// Does NOT shift the LCD register — the resolved pixel is stored in
/// the lcd_data_latch and shifted in later when a TOBA edge fires
/// (modeling the qp_ext_old lag on the final color output).
///
pub(in crate::ppu) fn resolve_current_pixel(
    bg_shifter: &BgShifter,
    obj_shifter: &ObjShifter,
    regs: &PipelineRegisters,
) -> PaletteIndex {
    let bgp = regs.palettes.background_for_bg_resolve();
    let obp0 = regs.palettes.sprite0.output();
    let obp1 = regs.palettes.sprite1.output();
    let bg_window_enabled = regs.bg_window_enabled_for_resolve();
    let sprites_enabled = regs.control.sprites_enabled();

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
