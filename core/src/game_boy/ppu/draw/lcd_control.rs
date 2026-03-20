use crate::game_boy::ppu::{types::palette::PaletteIndex, screen::Screen};

use super::lcd_shift_register::LcdShiftRegister;

/// Bit mask for XUGU NAND5 decode: PX bits 0+1+2+5+7 = 1+2+4+32+128 = 167.
/// WODU = AND2(!FEPO, !XUGU). XUGU is low (WODU fires) when all five bits set.
const XUGU_MASK: u8 = 0b1010_0111; // bits 0,1,2,5,7

/// LCD Control block (die page 24).
///
/// Owns the pixel X position counter (XEHO-SYBE), LCD clock gating
/// (WUSA NOR latch), POVA fine-match trigger, LCD shift register
/// (TAXA chain), and LCD data pin latch (REMY/RAVO).
///
/// Inputs: SACU (pixel clock edge from page 27), pixel data (from
/// pixel mux, page 35), POVA (fine scroll match), WEGO (from page 21).
/// Outputs: XUGU (pixel counter decode for WODU on page 21), TOBA
/// (gated LCD clock, returned from `rise()`).
pub(in crate::game_boy::ppu) struct LcdControl {
    /// Hardware pixel counter (XEHO-SYBE). Counts from 0 when the
    /// pixel clock starts after startup. Drives WODU (hblank gate)
    /// at PX=167. Not reset on window trigger — PX is a monotonic
    /// per-line counter.
    pixel_counter: u8,
    /// WUSA NOR latch — LCD clock gate. SET by XAJO (AND2 of pixel
    /// counter bits 0 and 3, first at PX=9). CLEAR by WEGO
    /// (= OR2(VID_RST, VOGA)). Gates TOBA (LCD clock pin).
    wusa: bool,
    /// POVA_FINE_MATCH_TRIGp — rising-edge trigger on the fine scroll
    /// match signal. Computed on rising phases as AND2(PUXA, !NYZE).
    /// Generates one extra LCD clock pulse via SEMU = OR2(TOBA, POVA),
    /// providing the 160th LCD clock edge before WUSA opens.
    pova: bool,
    /// LCD shift register — 159-stage pixel buffer between the pixel
    /// mux and the Screen. Replaces direct framebuffer writes.
    shift_register: LcdShiftRegister,
    /// LCD data pin latch (REMY/RAVO qp_ext_old model). On hardware,
    /// the LCD data pins are combinational from the pipe MSBs, but the
    /// LCD captures `qp_ext_old()` — the previous half-cycle's value.
    /// This buffer holds the resolved pixel from the previous SACU edge.
    /// TOBA shifts this buffered value into the LCD shift register,
    /// giving a 1-dot lag: TOBA at PX=N outputs PX=(N-1)'s pixel.
    data_latch: PaletteIndex,
}

impl LcdControl {
    pub(in crate::game_boy::ppu) fn new() -> Self {
        LcdControl {
            pixel_counter: 0,
            wusa: false,
            pova: false,
            shift_register: LcdShiftRegister::new(),
            data_latch: PaletteIndex(0),
        }
    }

    /// Rising edge: pixel counter increment, XAJO/WUSA set, TOBA
    /// shift, data latch update. All internal to LCD Control on the
    /// die — the caller provides SACU, the resolved pixel, and POVA.
    ///
    /// Returns TOBA (gated LCD clock) so the caller can gate
    /// window_zero_pixel consumption.
    pub(in crate::game_boy::ppu) fn rise(&mut self, sacu: bool, pixel: PaletteIndex, pova: bool) -> bool {
        // Pixel counter increment (SACU clock).
        if sacu {
            self.pixel_counter += 1;
        }

        // XAJO: AND2(PX bit 0, PX bit 3). Sets the WUSA NOR latch,
        // opening the LCD clock gate. First fires at PX=9 (0b1001).
        if !self.wusa && (self.pixel_counter & 0b1001 == 0b1001) {
            self.wusa = true;
        }

        // TOBA = AND2(WUSA, SACU) — the gated LCD clock.
        let toba = self.wusa && sacu;

        // LCD data pin lag: TOBA shifts the BUFFERED pixel (from the
        // previous SACU edge) into the shift register, then the latch
        // updates to the current pipe state. 1-dot offset: TOBA at
        // PX=9 outputs PX=8's pixel, etc.
        if toba {
            self.shift_register.shift_in(self.data_latch);
        }

        // Update the LCD data latch with the current pipe state.
        self.data_latch = pixel;

        // Store POVA.
        self.pova = pova;

        toba
    }

    /// Falling edge: WEGO = OR2(VID_RST, VOGA). When VOGA is set,
    /// clears WUSA. On the WODU dot (last_pixel), also
    /// shifts in the final pixel and latches the shift register to
    /// the screen.
    pub(in crate::game_boy::ppu) fn fall(&mut self, voga: bool, last_pixel: bool, screen: &mut Screen) {
        if !voga {
            return;
        }
        self.wusa = false;
        if last_pixel {
            self.shift_register.shift_in(self.data_latch);
            self.shift_register.latch_to_screen(screen);
        }
    }

    /// Update the LCD data latch directly. Used for out-of-band
    /// updates (SUZU/TEVO window tile load, sprite overwrite) that
    /// happen outside the normal SACU-driven `rise()` path.
    pub(in crate::game_boy::ppu) fn set_data_latch(&mut self, pixel: PaletteIndex) {
        self.data_latch = pixel;
    }

    /// XUGU decode: PX bits 0+1+2+5+7 all set (PX=167).
    /// Output signal from page 24 → page 21 for WODU computation.
    pub(in crate::game_boy::ppu) fn xugu(&self) -> bool {
        self.pixel_counter & XUGU_MASK == XUGU_MASK
    }

    /// Reset per-scanline state.
    pub(in crate::game_boy::ppu) fn reset(&mut self, scanline: u8) {
        self.pixel_counter = 0;
        self.wusa = false;
        self.pova = false;
        self.shift_register.reset(scanline);
        self.data_latch = PaletteIndex(0);
    }

    // --- Accessors ---

    pub(in crate::game_boy::ppu) fn pixel_counter(&self) -> u8 {
        self.pixel_counter
    }

    pub(in crate::game_boy::ppu) fn wusa(&self) -> bool {
        self.wusa
    }

    pub(in crate::game_boy::ppu) fn pova(&self) -> bool {
        self.pova
    }

    pub(in crate::game_boy::ppu) fn lcd_x(&self) -> u8 {
        self.shift_register.count()
    }

    pub(in crate::game_boy::ppu) fn data_latch_mut(&mut self) -> &mut PaletteIndex {
        &mut self.data_latch
    }
}
