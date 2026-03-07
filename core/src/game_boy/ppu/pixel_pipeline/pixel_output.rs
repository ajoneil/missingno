// --- Pixel mux (page 35 on the die) ---
//
// The pixel mux combines the BG and OBJ shift register outputs into a
// single color index, applies priority logic, maps through the
// appropriate palette, and writes the result to the screen.
//
// Pixel output is driven by SEMU edges — the true LCD clock signal:
//   SEMU = OR2(TOBA, POVA)
// POVA provides one edge at fine scroll match (the 160th clock edge,
// pre-shift pipe MSBs). TOBA provides 159 edges from PX=9 to PX=167
// (post-shift pipe MSBs). Total: 160 pixels per line.
//
// Sprite merge uses the data-pin model: when sprite data is merged into
// the pipe, the data pins update combinationally but no SEMU edge fires.
// The input latch of the LCD shift register is overwritten with
// post-merge data.

use crate::game_boy::ppu::{
    PipelineRegisters,
    palette::{PaletteIndex, PaletteMap},
};

use super::lcd_shift_register::LcdShiftRegister;
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

/// SEMU-edge pixel output (page 35 on the die).
///
/// Called when a SEMU edge fires — either POVA (even phase, pre-shift
/// pipe MSBs) or TOBA (odd phase, post-shift pipe MSBs). Reads the
/// current shift register MSBs, resolves the pixel through priority
/// logic and palette mapping, and shifts it into the LCD shift register.
///
/// Handles `window_zero_pixel`: when set, substitutes BG color 0
/// without reading the BG shifter. The OBJ shifter is still read
/// so sprite pixels mix against the zero background.
pub(super) fn semu_pixel_out(
    bg_shifter: &BgShifter,
    obj_shifter: &ObjShifter,
    shift_register: &mut LcdShiftRegister,
    window_zero_pixel: &mut bool,
    regs: &PipelineRegisters,
) {
    // Window reactivation zero pixel: substitute color 0 for the BG
    // pixel without popping the BG shifter. The OBJ shifter is still
    // popped so sprite pixels mix against the zero pixel.
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
                let mapped = sprite_palette.map(PaletteIndex(spr_color));
                shift_register.shift_in(mapped);
                return;
            }
        }

        let mapped = PaletteMap(regs.palettes.background.output()).map(PaletteIndex(bg_color));
        shift_register.shift_in(mapped);
        return;
    }

    let (bg_lo, bg_hi) = bg_shifter.read();
    let (spr_lo, spr_hi, spr_pal, spr_pri) = obj_shifter.read();
    let mapped = resolve_pixel(bg_lo, bg_hi, spr_lo, spr_hi, spr_pal, spr_pri, regs);
    shift_register.shift_in(mapped);
}

/// Data-pin pixel overwrite (sprite merge).
///
/// Called when sprite fetch completes and sprite data is merged into
/// the pipe. No SEMU edge fires during sprite fetch (SACU frozen →
/// TOBA=0), but the data pins (REMY/RAVO) update combinationally
/// from the pipe MSBs — now containing merged sprite data. The input
/// latch of the LCD shift register is overwritten with the resolved
/// pixel. Does not advance the shift register count.
///
/// Handles `window_zero_pixel`: if set, the last SEMU-written pixel
/// was the window reactivation zero pixel. The sprite merge overwrites
/// it with bg_color=0 + sprite mix (same as the original zero pixel
/// but with merged sprite data).
pub(super) fn sprite_overwrite_pixel_out(
    bg_shifter: &BgShifter,
    obj_shifter: &ObjShifter,
    shift_register: &mut LcdShiftRegister,
    window_zero_pixel: &mut bool,
    regs: &PipelineRegisters,
) {
    if shift_register.count() == 0 {
        return;
    }

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
                let mapped = sprite_palette.map(PaletteIndex(spr_color));
                shift_register.overwrite_input_latch(mapped);
                return;
            }
        }

        let mapped = PaletteMap(regs.palettes.background.output()).map(PaletteIndex(bg_color));
        shift_register.overwrite_input_latch(mapped);
        return;
    }

    let (bg_lo, bg_hi) = bg_shifter.read();
    let (spr_lo, spr_hi, spr_pal, spr_pri) = obj_shifter.read();
    let mapped = resolve_pixel(bg_lo, bg_hi, spr_lo, spr_hi, spr_pal, spr_pri, regs);
    shift_register.overwrite_input_latch(mapped);
}
