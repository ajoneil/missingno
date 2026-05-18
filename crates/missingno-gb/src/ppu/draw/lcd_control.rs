//! WUSA clock gating, POVA trigger, and LCD pixel push.

use crate::ppu::PixelOutput;
use crate::ppu::types::palette::PaletteIndex;

/// TOBA = AND2(WUSA, SACU) gates pixel emit; cp_pad waveform (SEMU = OR2(TOBA, POVA)) is not modelled.
pub(in crate::ppu) struct LcdControl {
    /// WUSA nor_latch: set by XAJO (PX bits 0&3, first at PX=9), cleared by WEGO=OR2(VID_RST, VOGA).
    wusa: bool,
    /// POVA fine-scroll match. Would feed SEMU=OR2(TOBA, POVA) on hardware (not wired; no cp_pad).
    pova: bool,
    lcd_push_count: u8,
    /// Latched at reset so pixel output uses the right Y after HBlank LY advance.
    scanline: u8,
}

impl LcdControl {
    pub(in crate::ppu) fn new() -> Self {
        LcdControl {
            wusa: false,
            pova: false,
            lcd_push_count: 0,
            scanline: 0,
        }
    }

    pub(in crate::ppu) fn post_boot() -> Self {
        LcdControl {
            wusa: false,
            pova: false,
            lcd_push_count: 160,
            scanline: 143,
        }
    }

    /// PPU fall: XAJO sets WUSA, TOBA pushes screen_x=0..158. Caller passes post-advance `px_value`.
    pub(in crate::ppu) fn on_ppu_clock_fall(
        &mut self,
        sacu: bool,
        pixel: PaletteIndex,
        pova: bool,
        px_value: u8,
    ) -> (bool, Option<PixelOutput>) {
        // XAJO = AND2(PX bit0, PX bit3); first fires at PX=9.
        if !self.wusa && (px_value & 0b1001 == 0b1001) {
            self.wusa = true;
        }

        let toba = self.wusa && sacu;

        let pixel_out = if toba
            && self.lcd_push_count < 159
            && self.scanline < crate::ppu::screen::NUM_SCANLINES
        {
            let out = PixelOutput {
                x: self.lcd_push_count,
                y: self.scanline,
                shade: pixel.0,
            };
            self.lcd_push_count += 1;
            Some(out)
        } else {
            None
        };

        self.pova = pova;

        (toba, pixel_out)
    }

    /// PPU rise: VOGA clears WUSA; WODU dot pushes screen_x=159 from the post-fall-shift shifter MSB.
    pub(in crate::ppu) fn on_ppu_clock_rise(
        &mut self,
        voga: bool,
        wodu: bool,
        post_shift_pixel: PaletteIndex,
    ) -> Option<PixelOutput> {
        let pixel_out = if wodu
            && self.lcd_push_count < crate::ppu::screen::PIXELS_PER_LINE
            && self.scanline < crate::ppu::screen::NUM_SCANLINES
        {
            let out = PixelOutput {
                x: self.lcd_push_count,
                y: self.scanline,
                shade: post_shift_pixel.0,
            };
            self.lcd_push_count += 1;
            Some(out)
        } else {
            None
        };

        if voga {
            self.wusa = false;
        }

        pixel_out
    }

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
    }

    pub(in crate::ppu) fn wusa(&self) -> bool {
        self.wusa
    }

    pub(in crate::ppu) fn pova(&self) -> bool {
        self.pova
    }

    pub(in crate::ppu) fn lcd_x(&self) -> u8 {
        self.lcd_push_count
    }
}
