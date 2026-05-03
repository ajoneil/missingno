//! LCD control: LCD clock gating and pixel data pin latch.
//!
//! The two edge methods (`on_ppu_clock_fall` / `on_ppu_clock_rise`)
//! dispatch multiple discrete concerns per edge — SACU-driven pixel
//! counter advance, WUSA latch management, TOBA gated-clock generation,
//! data latch rotation, and WODU-dot final-pixel push — so they are
//! signal-named rather than work-semantic.

use crate::ppu::types::palette::PaletteIndex;
use crate::ppu::PixelOutput;

/// LCD clock gating + LCD data pin latch.
///
/// Owns the LCD clock gate (hardware WUSA `nor_latch`), a POVA
/// fine-scroll-match trigger (consumed at the caller for LCD-clock
/// generation), and the LCD data pin latch (hardware REMY/RAVO
/// `qp_ext_old` model). The pixel X position counter (PX) lives in
/// its own module (see `pixel_counter.rs`).
///
/// Pixel output is returned as a [`PixelOutput`] signal rather than
/// written to an internal framebuffer — the caller (emulation loop)
/// is responsible for building whatever representation it needs.
///
/// Inputs: SACU (pixel clock edge), pixel data (from the pixel mux),
/// POVA (fine-scroll-match, from FineScroll), WEGO (OR2(VID_RST, VOGA)),
/// PX value (from PixelCounter, used for the WUSA XAJO gate).
/// Outputs: TOBA (gated LCD clock, returned from edge methods).
///
/// Hardware chain: WUSA `nor_latch` gates TOBA = AND2(WUSA, SACU).
/// SEMU = OR2(TOBA, POVA) would be the cp_pad pixel-clock source on
/// hardware (RYPO = NOT(SEMU) → cp_pad pad), but cp_pad waveform is
/// not currently modelled — POVA's contribution would shift its pixel
/// off the 159-stage LCD shift register per the collapsed-push model
/// (see `pixel_output.rs`). Adding cp_pad state awaits a consumer
/// (debugger panel or FST-comparison harness); tracked in the
/// alignment-log's LCD output alignment deferred arc.
pub(in crate::ppu) struct LcdControl {
    /// LCD clock gate latch (hardware WUSA `nor_latch`). SET by XAJO
    /// (AND2 of pixel-counter bits 0 and 3, first at PX=9). CLEAR by
    /// WEGO = OR2(VID_RST, VOGA). Gates TOBA = AND2(WUSA, SACU).
    wusa: bool,
    /// Fine-scroll-match trigger (hardware POVA). Computed by
    /// FineScroll as a rising-edge detector on PUXA; on hardware POVA
    /// = AND2(PUXA, NYZE) fires for ~1 dot. Would contribute to
    /// SEMU = OR2(TOBA, POVA) on hardware for an extra cp_pad clock
    /// edge per Mode 3 scanline — not currently wired (no cp_pad
    /// consumer; see LCD output alignment deferred arc).
    pova: bool,
    /// Number of pixels pushed to the LCD on this line. Replaces the
    /// old shift register's count — nothing reads intermediate pixel
    /// data, so only the count is needed for lcd_x tracking.
    lcd_push_count: u8,
    /// The scanline this line is rendering. Captured at reset time so
    /// pixel output uses the correct Y even if LY has incremented by
    /// HBlank. Matches the old shift register's scanline field.
    scanline: u8,
    /// LCD data pin latch (REMY/RAVO qp_ext_old model). On hardware,
    /// the LCD data pins are combinational from the pipe MSBs, but the
    /// LCD captures `qp_ext_old()` — the previous half-cycle's value.
    /// This buffer holds the resolved pixel from the previous SACU edge.
    /// TOBA shifts this buffered value to the LCD output, giving a
    /// 1-dot lag: TOBA at PX=N outputs PX=(N-1)'s pixel.
    data_latch: PaletteIndex,
}

impl LcdControl {
    pub(in crate::ppu) fn new() -> Self {
        LcdControl {
            wusa: false,
            pova: false,
            lcd_push_count: 0,
            scanline: 0,
            data_latch: PaletteIndex(0),
        }
    }

    /// Boot-ROM-handoff LCD control state: leftover counter
    /// values from the prior frame's last visible scanline (LY=143). The
    /// 160 visible pixels were pushed through TOBA before WODU fired and
    /// nothing resets `lcd_push_count` / `scanline` during VBlank — they
    /// persist until the next active-line `reset_scanline()` call clears
    /// them via `LcdControl::reset()`.
    pub(in crate::ppu) fn post_boot() -> Self {
        LcdControl {
            wusa: false,
            pova: false,
            lcd_push_count: 160,
            scanline: 143,
            data_latch: PaletteIndex(0),
        }
    }

    /// PPU clock fall (master-clock rise; gate: ALET falling): XAJO/WUSA
    /// set, TOBA pixel output, data latch update. Dispatcher for the
    /// SACU-driven pixel-output concerns on this edge — multiple
    /// unrelated effects at one edge. The caller advances `PixelCounter`
    /// first and passes in the post-advance `px_value` so WUSA's XAJO
    /// gate reads the current counter state.
    ///
    /// Returns `(toba, pixel_out)` where `toba` is the gated LCD clock
    /// and `pixel_out` is the pixel pushed to the LCD (if any).
    pub(in crate::ppu) fn on_ppu_clock_fall(
        &mut self,
        sacu: bool,
        pixel: PaletteIndex,
        pova: bool,
        px_value: u8,
    ) -> (bool, Option<PixelOutput>) {
        // XAJO: AND2(PX bit 0, PX bit 3). Sets the WUSA NOR latch,
        // opening the LCD clock gate. First fires at PX=9 (0b1001).
        if !self.wusa && (px_value & 0b1001 == 0b1001) {
            self.wusa = true;
        }

        // TOBA = AND2(WUSA, SACU) — the gated LCD clock.
        let toba = self.wusa && sacu;

        // LCD data pin lag: TOBA pushes the BUFFERED pixel (from the
        // previous SACU edge) to the LCD. 1-dot offset: TOBA at
        // PX=9 outputs PX=8's pixel, etc.
        //
        // On real hardware, TOBA fires one extra time after WODU
        // (WUSA is cleared by VOGA on the same dot). The 159-stage
        // shift register naturally absorbed this — the extra pixel
        // pushed the first (junk) pixel off the end. With direct
        // output we skip it: only the first 159 TOBA pixels are visible.
        let pixel_out = if toba
            && self.lcd_push_count < 159
            && self.scanline < crate::ppu::screen::NUM_SCANLINES
        {
            let out = PixelOutput {
                x: self.lcd_push_count,
                y: self.scanline,
                shade: self.data_latch.0,
            };
            self.lcd_push_count += 1;
            Some(out)
        } else {
            None
        };

        // Update the LCD data latch with the current pipe state.
        self.data_latch = pixel;

        // Store POVA.
        self.pova = pova;

        (toba, pixel_out)
    }

    /// PPU clock rise (master-clock fall; gate: ALET rising): WEGO =
    /// OR2(VID_RST, VOGA). When VOGA is set, clears WUSA. On the WODU
    /// dot (last_pixel), pushes the final pixel to the LCD. Two discrete
    /// concerns tied to the hblank transition — signal-named for the
    /// dispatch shape.
    ///
    /// Returns the pixel pushed to the LCD on the WODU dot (if any).
    pub(in crate::ppu) fn on_ppu_clock_rise(
        &mut self,
        voga: bool,
        wodu: bool,
    ) -> Option<PixelOutput> {
        // WODU fires combinationally on the dot pixel_counter reaches 167.
        // The final pixel push happens on the WODU dot, before VOGA
        // captures on the same falling phase.
        let pixel_out = if wodu
            && self.lcd_push_count < crate::ppu::screen::PIXELS_PER_LINE
            && self.scanline < crate::ppu::screen::NUM_SCANLINES
        {
            let out = PixelOutput {
                x: self.lcd_push_count,
                y: self.scanline,
                shade: self.data_latch.0,
            };
            self.lcd_push_count += 1;
            Some(out)
        } else {
            None
        };

        // WUSA is cleared by WEGO = OR2(VID_RST, VOGA). VOGA fires on
        // the same dot as WODU (half-cycle DFF17 delay via ALET falling).
        if voga {
            self.wusa = false;
        }

        pixel_out
    }

    /// Update the LCD data latch directly. Used for out-of-band
    /// updates (SUZU/TEVO window tile load, sprite overwrite) that
    /// happen outside the normal SACU-driven `rise()` path.
    pub(in crate::ppu) fn set_data_latch(&mut self, pixel: PaletteIndex) {
        self.data_latch = pixel;
    }

    /// Reset per-scanline state.
    pub(in crate::ppu) fn reset(&mut self, scanline: u8) {
        debug_assert!(
            self.lcd_push_count == 0 || self.lcd_push_count == 160,
            "lcd_push_count={} at reset (scanline {scanline}), expected 0 or 160",
            self.lcd_push_count,
        );
        self.wusa = false;
        self.pova = false;
        self.lcd_push_count = 0;
        self.scanline = scanline;
        self.data_latch = PaletteIndex(0);
    }

    // --- Accessors ---

    pub(in crate::ppu) fn wusa(&self) -> bool {
        self.wusa
    }

    pub(in crate::ppu) fn pova(&self) -> bool {
        self.pova
    }

    pub(in crate::ppu) fn lcd_x(&self) -> u8 {
        self.lcd_push_count
    }

    pub(in crate::ppu) fn data_latch_mut(&mut self) -> &mut PaletteIndex {
        &mut self.data_latch
    }
}
