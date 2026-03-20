use crate::game_boy::ppu::types::palette::PaletteIndex;
use crate::game_boy::ppu::screen::{PIXELS_PER_LINE, Screen};

/// 159-stage, 2-bit-wide LCD shift register with input latch.
///
/// Models the DMG LCD column driver input. On each SEMU clock edge,
/// data shifts through 159 stages and a new pixel enters the input
/// latch. At end-of-line, the 159 stages plus input latch (160 total)
/// transfer to the column drivers (the Screen).
///
/// The POVA pixel enters first, then 159 TOBA pixels shift it out.
/// After 160 clocks through 159 stages, the POVA pixel falls off
/// the far end. Only TOBA pixels remain — no special-case discard.
pub struct LcdShiftRegister {
    /// The 159 shift register stages plus 1 input latch = 160 slots.
    /// Index 0 is the output end (oldest pixel, shifts out first).
    /// Index 159 is the input latch (newest pixel).
    stages: [PaletteIndex; PIXELS_PER_LINE as usize],
    /// Number of SEMU clocks received on this line. Used to know
    /// how many valid pixels are in the register for the latch
    /// transfer, and replaces `lcd_x` for position tracking.
    count: u8,
    /// The scanline this register is accumulating for. Captured at
    /// reset time so latch_to_screen uses the correct line even if
    /// LY has incremented by HBlank.
    scanline: u8,
}

impl LcdShiftRegister {
    /// Create a new shift register, all stages initialized to palette index 0.
    pub fn new() -> Self {
        Self {
            stages: [PaletteIndex(0); PIXELS_PER_LINE as usize],
            count: 0,
            scanline: 0,
        }
    }

    /// SEMU clock edge: shift all stages left by one and write the new
    /// pixel into the input latch (last position). The oldest pixel
    /// falls off the output end.
    pub fn shift_in(&mut self, pixel: PaletteIndex) {
        self.stages.copy_within(1.., 0);
        self.stages[PIXELS_PER_LINE as usize - 1] = pixel;
        self.count += 1;
    }

    /// Number of SEMU clocks received. Replaces `lcd_x` for pixel
    /// position tracking (sprite trigger matching, WODU gating, etc.).
    pub fn count(&self) -> u8 {
        self.count
    }

    /// PIN_55 LCD_LATCH: transfer shift register contents to the Screen
    /// for the given scanline. The 160 stages (159 register + input
    /// latch) map directly to 160 screen columns.
    ///
    /// After 160 SEMU clocks, the POVA pixel has shifted out and the
    /// register holds exactly 160 displayable pixels. If fewer than
    /// 160 clocks fired (abnormal line termination), only the rightmost
    /// `count` pixels are valid; earlier stages contain stale data.
    pub fn latch_to_screen(&self, screen: &mut Screen) {
        if self.scanline >= crate::game_boy::ppu::screen::NUM_SCANLINES {
            return;
        }
        for x in 0..PIXELS_PER_LINE {
            screen.set_pixel(x, self.scanline, self.stages[x as usize]);
        }
    }

    /// Reset for a new scanline.
    pub fn reset(&mut self, scanline: u8) {
        self.count = 0;
        self.scanline = scanline;
        // Stage contents don't need clearing -- they'll be overwritten
        // by shift_in before latch_to_screen reads them.
    }
}
