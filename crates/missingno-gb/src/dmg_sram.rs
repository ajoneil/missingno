//! DMG on-die SRAM power-on pattern.
//!
//! Models the bring-up state of the SRAM cells used for WRAM (0xC000),
//! HRAM (0xFF80), and OAM (0xFE00). At power-on each cell settles
//! according to its bit-line / word-line layout: Gekkio's DMG dumps
//! show this as 16-byte word-line stripes alternating 0xFF and 0x00,
//! with a small minority of "stuck" cells that come up at values
//! outside {0x00, 0xFF} due to per-cell manufacturing variability.
//!
//! `fill` produces a deterministic approximation of that state: the
//! stripe pattern overlaid with a fixed table of stuck-cell offsets.
//! The pattern is purely a function of byte offset, so replay and
//! snapshot semantics are unaffected.

const STRIPE_LEN: usize = 16;

/// Stuck-cell overlay: byte offsets within an SRAM region that settle
/// outside the {0x00, 0xFF} stripe pattern. Offsets past the region's
/// length are silently ignored, so the same table works for WRAM
/// (8 KiB), OAM (160 B) and HRAM (127 B). Every supported region size
/// is large enough to hit at least one entry — the smallest offset
/// here is below 127 so HRAM is covered. Values are picked to be
/// mixed-bit so they are visibly non-canonical in a memory viewer.
const STUCK_CELLS: &[(usize, u8)] = &[
    (0x0011, 0x55),
    (0x0023, 0xAA),
    (0x0041, 0x42),
    (0x0067, 0x80),
    (0x007E, 0x02),
    (0x0098, 0xC3),
    (0x0103, 0x18),
    (0x01F4, 0x69),
    (0x0500, 0x33),
    (0x0A2C, 0xCC),
    (0x1000, 0x5A),
    (0x1FF0, 0xA5),
];

pub fn fill(buf: &mut [u8]) {
    for (i, byte) in buf.iter_mut().enumerate() {
        *byte = if (i / STRIPE_LEN) & 1 == 0 {
            0xFF
        } else {
            0x00
        };
    }
    for &(offset, value) in STUCK_CELLS {
        if let Some(slot) = buf.get_mut(offset) {
            *slot = value;
        }
    }
}
