//! PPU pixel counter (PX) — the pixel-position counter within a Mode 3
//! scanline.
//!
//! Hardware: 8-bit hybrid synchronous/ripple counter (XEHO/SAVY/XODU/XYDO
//! lower nibble clocked directly by SACU with XOR-carry; TUHU/TUKY/TAKO/SYBE
//! upper nibble clocked by TOCA = NOT(XYDO) rippling off bit 3). The bit-
//! level structure produces output equivalent to a u8 increment; the
//! emulator collapses it into a single `u8` field (honest-abstraction).
//!
//! Reset: TADY = NOR2(TOFU, ATEJ) drives the counter's reset during
//! scanline boundaries and VID_RST. `reset()` models the combined path.
//!
//! Terminal detection: XUGU (NAND5 over bits 0, 1, 2, 5, 7) fires at
//! PX=167, driving WODU → VOGA → WEGO → XYMU (Mode 3→0 chain, §2.4).
//! The `terminal()` method returns `true` at PX=167 — polarity-positive
//! semantic (matches XANO = NOT(XUGU)) rather than XUGU's active-low
//! hardware output.

/// XUGU NAND5 decode pattern: PX bits 0+1+2+5+7 all set = 0b10100111 = 167.
const TERMINAL_MASK: u8 = 0b1010_0111;

pub(in crate::ppu) struct PixelCounter(u8);

impl PixelCounter {
    pub(in crate::ppu) fn new() -> Self {
        Self(0)
    }

    /// Advance by one pixel. Callers gate on SACU (pixel-clock rising edge).
    pub(in crate::ppu) fn advance(&mut self) {
        self.0 += 1;
    }

    /// Scanline reset (TADY chain — shared with VOGA reset §2.4 and scan-
    /// counter reset §6.4.1 via ATEJ). Called at scanline boundaries and
    /// LCD-off paths.
    pub(in crate::ppu) fn reset(&mut self) {
        self.0 = 0;
    }

    /// Terminal-count decode. True at PX=167, matching XANO = NOT(XUGU)
    /// polarity (positive-at-terminal); the hardware XUGU NAND5 output is
    /// active-low. Feeds WODU via XANO per §2.14.2, triggering the Mode
    /// 3→0 chain per §2.4.
    pub(in crate::ppu) fn terminal(&self) -> bool {
        self.0 & TERMINAL_MASK == TERMINAL_MASK
    }

    pub(in crate::ppu) fn value(&self) -> u8 {
        self.0
    }
}
