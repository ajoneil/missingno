// Pixel mux: combines BG and OBJ shift register outputs into a single
// colour index, applies sprite priority, and maps through the active
// palette. Hardware chain: BG plane gates via LCDC.0 (VYXE) at RAJY /
// TADE; sprite priority MUX via RYFU / XULA / WOXA / NULY; palette
// lookup via AO2222 combiners MOKA / NURA / WUFU combined in PATY
// (OR3); final pad drivers RAVO (LD1) and REMY (LD0).
//
// LCD data pin lag (REMY/RAVO qp_ext_old model): the LCD data pins
// update combinationally from the pipe MSBs but the LCD captures the
// previous half-cycle's value. Each TOBA edge shifts the PREVIOUS
// dot's pixel into the LCD shift register, giving a 1-dot offset.
// 159 TOBA edges (PX=9–167) output pixels for PX=8–166; the 160th
// pixel (PX=167) is captured by the NOR latch at end-of-line. POVA
// fires once per scanline's fine-scroll-match timing but its pixel
// is shifted off by the 160 subsequent pushes (159 TOBA + 1 NOR
// latch) — observation-equivalent to a collapsed single-push model.
//
// Sprite merge updates the lcd_data_latch combinationally (no SEMU
// edge), so the next TOBA captures post-merge sprite data.

use crate::ppu::{
    types::palette::{PaletteIndex, PaletteMap},
    PipelineRegisters,
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
/// Handles `window_zero_pixel`: when set, substitutes BG color 0
/// without reading the BG shifter. The OBJ shifter is still read
/// so sprite pixels mix against the zero background.
pub(in crate::ppu) fn resolve_current_pixel(
    bg_shifter: &BgShifter,
    obj_shifter: &ObjShifter,
    window_zero_pixel: &mut bool,
    regs: &PipelineRegisters,
) -> PaletteIndex {
    let bgp = regs.palettes.background.output();
    let obp0 = regs.palettes.sprite0.output();
    let obp1 = regs.palettes.sprite1.output();
    let bg_window_enabled = regs.control.background_and_window_enabled();
    let sprites_enabled = regs.control.sprites_enabled();

    if *window_zero_pixel {
        *window_zero_pixel = false;
        let (spr_lo, spr_hi, spr_pal, spr_pri) = obj_shifter.read();
        let bg_color: u8 = 0;

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

        return PaletteMap(bgp).map(PaletteIndex(bg_color));
    }

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

/// Data-pin pixel overwrite (sprite merge).
///
/// Called when sprite fetch completes and sprite data is merged into
/// the pipe. No SEMU edge fires during sprite fetch (SACU frozen →
/// TOBA=0), but the data pins (REMY/RAVO) update combinationally
/// from the pipe MSBs — now containing merged sprite data. Updates
/// the lcd_data_latch so the next TOBA edge captures the post-merge
/// pixel instead of the pre-merge BG-only data.
pub(in crate::ppu) fn sprite_overwrite_data_latch(
    bg_shifter: &BgShifter,
    obj_shifter: &ObjShifter,
    lcd_data_latch: &mut PaletteIndex,
    window_zero_pixel: &mut bool,
    regs: &PipelineRegisters,
) {
    let bgp = regs.palettes.background.output();
    let obp0 = regs.palettes.sprite0.output();
    let obp1 = regs.palettes.sprite1.output();
    let bg_window_enabled = regs.control.background_and_window_enabled();
    let sprites_enabled = regs.control.sprites_enabled();

    let (spr_lo, spr_hi, spr_pal, spr_pri) = obj_shifter.read();

    if *window_zero_pixel {
        *window_zero_pixel = false;
        let bg_color: u8 = 0;

        if sprites_enabled {
            let spr_color = (spr_hi << 1) | spr_lo;
            if spr_color != 0 && (spr_pri == 0 || bg_color == 0) {
                let sprite_palette = if spr_pal == 0 {
                    PaletteMap(obp0)
                } else {
                    PaletteMap(obp1)
                };
                *lcd_data_latch = sprite_palette.map(PaletteIndex(spr_color));
                return;
            }
        }

        *lcd_data_latch = PaletteMap(bgp).map(PaletteIndex(bg_color));
        return;
    }

    let (bg_lo, bg_hi) = bg_shifter.read();
    *lcd_data_latch = resolve_pixel(
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
    );
}
