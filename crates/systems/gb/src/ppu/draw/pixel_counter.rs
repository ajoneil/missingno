//! PX counter (8-bit, SACU-clocked). Collapses XEHO/SAVY/XODU/XYDO + TUHU/TUKY/TAKO/SYBE into a u8.

/// XUGU NAND5 decode: PX bits 0+1+2+5+7 = 167.
const TERMINAL_MASK: u8 = 0b1010_0111;

pub(in crate::ppu) struct PixelCounter(u8);

impl PixelCounter {
    pub(in crate::ppu) fn new() -> Self {
        Self(0)
    }

    pub(in crate::ppu) fn post_boot() -> Self {
        Self(167)
    }

    pub(in crate::ppu) fn advance(&mut self) {
        self.0 += 1;
    }

    /// TADY chain (shared with VOGA / scan-counter resets).
    pub(in crate::ppu) fn reset(&mut self) {
        self.0 = 0;
    }

    /// True at PX=167 — XANO polarity, drives WODU → VOGA → WEGO → XYMU.
    pub(in crate::ppu) fn terminal(&self) -> bool {
        self.0 & TERMINAL_MASK == TERMINAL_MASK
    }

    pub(in crate::ppu) fn value(&self) -> u8 {
        self.0
    }
}
