//! DMG on-die SRAM power-on pattern: 16-byte word-line stripes
//! alternating 0xFF / 0x00, overlaid with a fixed table of "stuck"
//! cells that settle at mixed-bit values due to per-cell manufacturing
//! variability. Used to seed WRAM, HRAM, and OAM at cold start.

const STRIPE_LEN: usize = 16;

/// Offsets past the buffer length are ignored, so the same table seeds
/// WRAM (8 KiB), OAM (160 B), and HRAM (127 B). At least one entry
/// sits below 127 so HRAM is covered.
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
