//! WUSA clock gating, POVA trigger, and LCD pixel push.

use crate::ppu::DrawnPixel;

/// TOBA = AND2(WUSA, SACU) gates pixel emit; cp_pad waveform (SEMU = OR2(TOBA, POVA)) is not modelled.
pub(in crate::ppu) struct LcdControl {
    /// WUSA nor_latch: set by XAJO (PX bits 0&3, first at PX=9), cleared by WEGO=OR2(VID_RST, VOGA).
    pixel_gate: bool,
    /// POVA fine-scroll match. Would feed SEMU=OR2(TOBA, POVA) on hardware (not wired; no cp_pad).
    fine_scroll_match: bool,
    lcd_push_count: u8,
    /// Latched at reset so pixel output uses the right Y after HBlank LY advance.
    scanline: u8,
}

impl LcdControl {
    pub(in crate::ppu) fn new() -> Self {
        LcdControl {
            pixel_gate: false,
            fine_scroll_match: false,
            lcd_push_count: 0,
            scanline: 0,
        }
    }

    pub(in crate::ppu) fn post_boot() -> Self {
        LcdControl {
            pixel_gate: false,
            fine_scroll_match: false,
            lcd_push_count: 160,
            scanline: 143,
        }
    }

    /// PPU fall: XAJO sets WUSA, TOBA pushes screen_x=0..158. Caller passes post-advance `px_value`.
    pub(in crate::ppu) fn on_ppu_clock_fall<Pix: Copy>(
        &mut self,
        sacu: bool,
        pixel: Pix,
        fine_scroll_match: bool,
        px_value: u8,
    ) -> (bool, Option<DrawnPixel<Pix>>) {
        // XAJO = AND2(PX bit0, PX bit3); first fires at PX=9.
        if !self.pixel_gate && (px_value & 0b1001 == 0b1001) {
            self.pixel_gate = true;
        }

        let toba = self.pixel_gate && sacu;

        let pixel_out = if toba
            && self.lcd_push_count < 159
            && self.scanline < crate::ppu::screen::NUM_SCANLINES
        {
            let out = DrawnPixel {
                x: self.lcd_push_count,
                y: self.scanline,
                color: pixel,
            };
            self.lcd_push_count += 1;
            Some(out)
        } else {
            None
        };

        self.fine_scroll_match = fine_scroll_match;

        (toba, pixel_out)
    }

    /// PPU rise: VOGA clears WUSA; WODU dot pushes screen_x=159 from the post-fall-shift shifter MSB.
    pub(in crate::ppu) fn on_ppu_clock_rise<Pix: Copy>(
        &mut self,
        end_of_line_latched: bool,
        end_of_line: bool,
        post_shift_pixel: Pix,
    ) -> Option<DrawnPixel<Pix>> {
        let pixel_out = if end_of_line
            && self.lcd_push_count < crate::ppu::screen::PIXELS_PER_LINE
            && self.scanline < crate::ppu::screen::NUM_SCANLINES
        {
            let out = DrawnPixel {
                x: self.lcd_push_count,
                y: self.scanline,
                color: post_shift_pixel,
            };
            self.lcd_push_count += 1;
            Some(out)
        } else {
            None
        };

        if end_of_line_latched {
            self.pixel_gate = false;
        }

        pixel_out
    }

    pub(in crate::ppu) fn reset(&mut self, scanline: u8) {
        debug_assert!(
            self.lcd_push_count == 0 || self.lcd_push_count == 160,
            "lcd_push_count={} at reset (scanline {scanline}), expected 0 or 160",
            self.lcd_push_count,
        );
        self.pixel_gate = false;
        self.fine_scroll_match = false;
        self.lcd_push_count = 0;
        self.scanline = scanline;
    }

    pub(in crate::ppu) fn pixel_gate(&self) -> bool {
        self.pixel_gate
    }

    pub(in crate::ppu) fn fine_scroll_match(&self) -> bool {
        self.fine_scroll_match
    }

    pub(in crate::ppu) fn lcd_x(&self) -> u8 {
        self.lcd_push_count
    }
}
