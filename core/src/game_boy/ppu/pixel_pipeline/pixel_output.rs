// --- Pixel mux (page 35 on the die) ---
//
// The pixel mux combines the BG and OBJ shift register outputs into a
// single color index, applies priority logic, maps through the
// appropriate palette, and writes the result to the screen.
//
// LCD data pin lag model (REMY/RAVO qp_ext_old):
//   The LCD data pins update combinationally from the pipe MSBs every
//   phase, but the LCD captures qp_ext_old — the previous half-cycle's
//   value. This means each TOBA edge shifts the PREVIOUS dot's pixel
//   into the LCD shift register, giving a 1-dot offset.
//
//   159 TOBA edges (PX=9–167) output pixels for PX=8–166.
//   The 160th pixel (PX=167) is captured by the NOR latch at EOL.
//   POVA fires for timing but its pixel is pushed off the register
//   by the 160 subsequent pixels (159 TOBA + 1 NOR latch).
//
// Sprite merge updates the lcd_data_latch combinationally (no SEMU
// edge), so the next TOBA captures post-merge sprite data.

use crate::game_boy::ppu::{
    PipelineRegisters,
    palette::{PaletteIndex, PaletteMap},
};

use super::shifters::{BgShifter, ObjShifter};

/// Resolve BG and OBJ pixel values into a final palette index through
/// priority logic and palette mapping.
fn resolve_pixel(
    bg_lo: u8,
    bg_hi: u8,
    spr_lo: u8,
    spr_hi: u8,
    spr_pal: u8,
    spr_pri: u8,
    regs: &PipelineRegisters,
) -> PaletteIndex {
    // Form 2-bit BG color index (0 if BG/window disabled via LCDC.0)
    let bg_color = if regs.control.background_and_window_enabled() {
        (bg_hi << 1) | bg_lo
    } else {
        0
    };

    // Sprite priority mixing
    if regs.control.sprites_enabled() {
        let spr_color = (spr_hi << 1) | spr_lo;
        if spr_color != 0 && (spr_pri == 0 || bg_color == 0) {
            let sprite_palette = if spr_pal == 0 {
                PaletteMap(regs.palettes.sprite0.output())
            } else {
                PaletteMap(regs.palettes.sprite1.output())
            };
            return sprite_palette.map(PaletteIndex(spr_color));
        }
    }

    // Background pixel
    PaletteMap(regs.palettes.background.output()).map(PaletteIndex(bg_color))
}

/// Resolve the current pipe MSBs into a palette index for the LCD
/// data latch (REMY/RAVO). Does NOT shift the LCD register — the
/// resolved pixel is stored in the lcd_data_latch and shifted in
/// later when a TOBA edge fires (modeling the qp_ext_old lag).
///
/// Handles `window_zero_pixel`: when set, substitutes BG color 0
/// without reading the BG shifter. The OBJ shifter is still read
/// so sprite pixels mix against the zero background.
pub(super) fn resolve_current_pixel(
    bg_shifter: &BgShifter,
    obj_shifter: &ObjShifter,
    window_zero_pixel: &mut bool,
    regs: &PipelineRegisters,
) -> PaletteIndex {
    if *window_zero_pixel {
        *window_zero_pixel = false;
        let (spr_lo, spr_hi, spr_pal, spr_pri) = obj_shifter.read();
        let bg_color: u8 = 0;

        if regs.control.sprites_enabled() {
            let spr_color = (spr_hi << 1) | spr_lo;
            if spr_color != 0 && (spr_pri == 0 || bg_color == 0) {
                let sprite_palette = if spr_pal == 0 {
                    PaletteMap(regs.palettes.sprite0.output())
                } else {
                    PaletteMap(regs.palettes.sprite1.output())
                };
                return sprite_palette.map(PaletteIndex(spr_color));
            }
        }

        return PaletteMap(regs.palettes.background.output()).map(PaletteIndex(bg_color));
    }

    let (bg_lo, bg_hi) = bg_shifter.read();
    let (spr_lo, spr_hi, spr_pal, spr_pri) = obj_shifter.read();
    resolve_pixel(bg_lo, bg_hi, spr_lo, spr_hi, spr_pal, spr_pri, regs)
}

/// Data-pin pixel overwrite (sprite merge).
///
/// Called when sprite fetch completes and sprite data is merged into
/// the pipe. No SEMU edge fires during sprite fetch (SACU frozen →
/// TOBA=0), but the data pins (REMY/RAVO) update combinationally
/// from the pipe MSBs — now containing merged sprite data. Updates
/// the lcd_data_latch so the next TOBA edge captures the post-merge
/// pixel instead of the pre-merge BG-only data.
pub(super) fn sprite_overwrite_data_latch(
    bg_shifter: &BgShifter,
    obj_shifter: &ObjShifter,
    lcd_data_latch: &mut PaletteIndex,
    window_zero_pixel: &mut bool,
    regs: &PipelineRegisters,
) {
    *lcd_data_latch = resolve_current_pixel(bg_shifter, obj_shifter, window_zero_pixel, regs);
}
