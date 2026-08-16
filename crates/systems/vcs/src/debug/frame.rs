//! Turning the core's emitted scanlines into a displayable field: the picture
//! window a television crops to, the palette that colours it, and the pacing
//! the broadcast standard implies.

use std::time::Duration;

use missingno_core::video::IndexedFrame;
use rgb::RGB8;

use crate::TvStandard;
use crate::tia::{VISIBLE_CLOCKS, palette_index};

/// Frames are emergent from VSYNC; bound the search so a kernel that never
/// syncs cannot stall the emulation thread.
pub(super) const FRAME_BUDGET_LINES: usize = 1000;

/// Scanlines of asserted VSYNC the television integrates before the field
/// re-anchors. The console drives VSYNC as a plain latch; this lock lives in
/// the set (off-chip) and is calibratable — reference emulators model 2 and the
/// safe kernel convention is 3, so anything shorter is swallowed.
pub(super) const VSYNC_LOCK_LINES: usize = 2;

/// Nominal frame: a full field of 228-clock lines at the colour clock — 262
/// lines (NTSC) or 312 (PAL). Kernels vary line counts; pacing uses the
/// convention so the frame rate follows the broadcast standard.
pub(super) fn frame_interval(standard: TvStandard) -> Duration {
    let lines = match standard {
        TvStandard::Ntsc => 262.0,
        TvStandard::Pal | TvStandard::Secam => 312.0,
    };
    Duration::from_secs_f32(lines * 228.0 / crate::tv_standard::master_clock_hz(standard) as f32)
}

/// The picture window shown from the full field the core emits: skip the
/// VBLANK lead-in after VSYNC, then show a fixed height so on-screen
/// geometry stays stable across kernels of varying line count. Values are
/// the standard NTSC/PAL picture regions (a TV crops to roughly this).
/// Frontend-only — the core keeps emitting every scanline.
struct DisplayWindow {
    skip: usize,
    height: usize,
}

fn display_window(standard: TvStandard) -> DisplayWindow {
    match standard {
        TvStandard::Ntsc => DisplayWindow {
            skip: 23,
            height: 228,
        },
        // SECAM shares PAL's 50 Hz, 312-line field geometry.
        TvStandard::Pal | TvStandard::Secam => DisplayWindow {
            skip: 32,
            height: 274,
        },
    }
}

pub(super) fn indexed_frame(lines: &[[u8; VISIBLE_CLOCKS]], standard: TvStandard) -> IndexedFrame {
    let window = display_window(standard);
    let black = palette_index(0) as u8;
    let mut pixels = vec![black; window.height * VISIBLE_CLOCKS];
    for row in 0..window.height {
        if let Some(line) = lines.get(window.skip + row) {
            let dst = row * VISIBLE_CLOCKS;
            for (i, &p) in line.iter().enumerate() {
                pixels[dst + i] = palette_index(p) as u8;
            }
        }
    }
    IndexedFrame {
        width: VISIBLE_CLOCKS as u32,
        height: window.height as u32,
        pixels: pixels.into(),
        palette: region_palette(standard),
    }
}

pub(super) fn blank_frame() -> IndexedFrame {
    let height = display_window(TvStandard::Ntsc).height as u32;
    IndexedFrame::blank(
        VISIBLE_CLOCKS as u32,
        height,
        region_palette(TvStandard::Ntsc),
    )
}

/// The core's TIA palette for a standard as the screen path's shared RGB8 slice
/// — NTSC/PAL hue decode, or SECAM's luma-only 8 colours.
fn region_palette(standard: TvStandard) -> std::sync::Arc<[RGB8]> {
    use std::sync::OnceLock;
    static PALETTES: OnceLock<[std::sync::Arc<[RGB8]>; 3]> = OnceLock::new();
    let build = |standard| -> std::sync::Arc<[RGB8]> {
        crate::tia::palette(standard)
            .iter()
            .map(|&(r, g, b)| RGB8::new(r, g, b))
            .collect::<Vec<_>>()
            .into()
    };
    let cache = PALETTES.get_or_init(|| {
        [
            build(TvStandard::Ntsc),
            build(TvStandard::Pal),
            build(TvStandard::Secam),
        ]
    });
    let index = match standard {
        TvStandard::Ntsc => 0,
        TvStandard::Pal => 1,
        TvStandard::Secam => 2,
    };
    cache[index].clone()
}
