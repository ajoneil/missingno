// --- Pixel mux (page 35 on the die) ---
//
// The pixel mux combines the BG and OBJ shift register outputs into a
// single color index, applies priority logic, maps through the
// appropriate palette, and writes the result to the screen.

use crate::game_boy::ppu::{
    PipelineRegisters, VideoControl,
    palette::{PaletteIndex, PaletteMap},
    screen::{self, Screen},
};

use super::FIRST_VISIBLE_PIXEL;
use super::fine_scroll::FineScroll;
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

/// Main pixel output path (page 35 on the die).
///
/// Reads the MSB from each shift register (already shifted in
/// mode3_odd), forms the 2-bit color indices, applies priority
/// logic, selects the winning pixel, and maps it through the
/// appropriate palette to the LCD. The pixel counter has already
/// been incremented before this call — lcd_x is derived from the
/// post-increment value.
pub(super) fn shift_pixel_out(
    bg_shifter: &BgShifter,
    obj_shifter: &ObjShifter,
    fine_scroll: &FineScroll,
    pixel_counter: u8,
    window_zero_pixel: &mut bool,
    screen: &mut Screen,
    regs: &PipelineRegisters,
    video: &VideoControl,
) {
    // Window reactivation zero pixel: substitute color 0 for the BG
    // pixel without popping the BG shifter. The OBJ shifter is still
    // popped so sprite pixels mix against the zero pixel.
    if *window_zero_pixel {
        *window_zero_pixel = false;
        let (spr_lo, spr_hi, spr_pal, spr_pri) = obj_shifter.read();

        if !fine_scroll.pixel_clock_active() {
            return;
        }
        if pixel_counter < FIRST_VISIBLE_PIXEL {
            return;
        }
        if pixel_counter >= FIRST_VISIBLE_PIXEL + screen::PIXELS_PER_LINE {
            return;
        }

        let x = pixel_counter - FIRST_VISIBLE_PIXEL;
        let y = video.ly();
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
                screen.set_pixel(x, y, mapped);
                return;
            }
        }

        let mapped = PaletteMap(regs.palettes.background.output()).map(PaletteIndex(bg_color));
        screen.set_pixel(x, y, mapped);
        return;
    }

    // Shift registers have already been advanced in mode3_odd
    // (SACU clock edge fires before LOZE load). Read the post-
    // shift/post-load MSB for pixel output.
    let (bg_lo, bg_hi) = bg_shifter.read();
    let (spr_lo, spr_hi, spr_pal, spr_pri) = obj_shifter.read();

    // During fine scroll gating (ROXY active), the pixel clock is
    // frozen on hardware — no LCD output. The shifters already
    // advanced in mode3_odd (they shift regardless of fine scroll).
    if !fine_scroll.pixel_clock_active() {
        return;
    }

    // PX 1 through 7 are invisible — the first tile shifts through the
    // pipe without writing to the framebuffer. The WUSA LCD gate opens
    // at PX=8 (FIRST_VISIBLE_PIXEL), producing lcd_x = 0.
    if pixel_counter < FIRST_VISIBLE_PIXEL {
        return;
    }

    // Past the visible region — safety guard for dots between WODU
    // and rendering latch clearing.
    if pixel_counter >= FIRST_VISIBLE_PIXEL + screen::PIXELS_PER_LINE {
        return;
    }

    let x = pixel_counter - FIRST_VISIBLE_PIXEL;
    let y = video.ly();

    let mapped = resolve_pixel(bg_lo, bg_hi, spr_lo, spr_hi, spr_pal, spr_pri, regs);
    screen.set_pixel(x, y, mapped);
}

/// Pixel output without pipe shift (sfetch_done dot).
///
/// On hardware, pixel output fires every dot, reading the pipe MSBs and
/// writing to lcd_x derived from the pixel counter. On the sfetch_done
/// dot, the pipes do NOT shift (FEPO blocks clkpipe_gate), but pixel
/// output still fires. The pixel counter holds the same post-increment
/// value used by the trigger dot's pixel output (no increment occurs
/// during sprite fetch), so `lcd_x = pixel_counter - FIRST_VISIBLE_PIXEL`
/// directly gives the trigger dot's screen position.
pub(super) fn peek_pixel_out(
    bg_shifter: &BgShifter,
    obj_shifter: &ObjShifter,
    fine_scroll: &FineScroll,
    pixel_counter: u8,
    screen: &mut Screen,
    regs: &PipelineRegisters,
    video: &VideoControl,
) {
    let (bg_lo, bg_hi) = bg_shifter.read();
    let (spr_lo, spr_hi, spr_pal, spr_pri) = obj_shifter.read();

    if !fine_scroll.pixel_clock_active() {
        return;
    }
    if pixel_counter < FIRST_VISIBLE_PIXEL {
        return;
    }
    if pixel_counter >= FIRST_VISIBLE_PIXEL + screen::PIXELS_PER_LINE {
        return;
    }

    let x = pixel_counter - FIRST_VISIBLE_PIXEL;
    let y = video.ly();

    let mapped = resolve_pixel(bg_lo, bg_hi, spr_lo, spr_hi, spr_pal, spr_pri, regs);
    screen.set_pixel(x, y, mapped);
}
