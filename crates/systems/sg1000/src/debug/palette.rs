//! The chip stops at colour indices, so resolving them to RGB is presentation
//! policy the console states — through the chip crate's datasheet table.

use std::sync::{Arc, OnceLock};

use missingno_ti_vdp::PALETTE;
use rgb::RGB8;

/// The palette every frame and every graphics surface hands out, built once.
pub(crate) fn ti_palette() -> Arc<[RGB8]> {
    static COLOURS: OnceLock<Arc<[RGB8]>> = OnceLock::new();
    COLOURS
        .get_or_init(|| PALETTE.iter().map(|&rgb| RGB8::from(rgb)).collect())
        .clone()
}

/// One colour index resolved; index 0 is the all-planes-transparent
/// external-video pass-through and presents black.
pub(crate) fn ti_colour(index: u8) -> RGB8 {
    RGB8::from(PALETTE[index as usize & 0x0F])
}
