//! Console-agnostic video vocabulary: the frame formats a core hands the
//! frontend to display, and how it describes its video output.

use std::sync::Arc;

use rgb::RGB8;

use crate::tv::TvStandard;

/// A frame of palette indices plus the palette to resolve them with,
/// converted to RGBA at draw time. Height is per-frame: systems without a
/// hardware frame (emergent sync) legitimately vary line counts. The
/// palette is shared, not static — systems with programmable colour RAM
/// send the palette as it stood when the frame completed.
#[derive(Clone, Debug)]
pub struct IndexedFrame {
    pub width: u32,
    pub height: u32,
    /// Row-major palette indices, `width * height` entries.
    pub pixels: Arc<[u8]>,
    pub palette: Arc<[RGB8]>,
    /// How wide one pixel displays relative to its height — a display-side
    /// calibratable stage, derived from the system's dot clock on an NTSC
    /// 4:3 screen.
    pub pixel_aspect: f32,
}

impl IndexedFrame {
    pub fn blank(width: u32, height: u32, pixel_aspect: f32, palette: Arc<[RGB8]>) -> Self {
        IndexedFrame {
            width,
            height,
            pixels: vec![0; (width * height) as usize].into(),
            palette,
            pixel_aspect,
        }
    }

    pub fn to_rgba(&self) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
        for &index in self.pixels.iter() {
            let color = self
                .palette
                .get(index as usize)
                .copied()
                .unwrap_or(RGB8::new(0, 0, 0));
            rgba.extend_from_slice(&[color.r, color.g, color.b, 255]);
        }
        rgba
    }
}

/// A display-ready RGBA frame — colours already resolved.
#[derive(Clone, Debug)]
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
    pub pixel_aspect: f32,
}

impl RgbaFrame {
    /// A blank white frame at the given dimensions.
    pub fn blank(width: u32, height: u32) -> Self {
        RgbaFrame {
            width,
            height,
            pixels: vec![255; (width * height * 4) as usize].into(),
            pixel_aspect: 1.0,
        }
    }
}

/// A console frame whose final colours are frontend policy applied at draw
/// time; `resolve_rgba` is the policy-free default rendering.
pub trait ConsoleFrame: Send + Sync {
    fn as_any(&self) -> &dyn std::any::Any;
    fn resolve_rgba(&self) -> RgbaFrame;
}

/// One completed frame in whichever form its core produces it.
pub enum Frame {
    Indexed(IndexedFrame),
    Rgba(RgbaFrame),
    Console(Box<dyn ConsoleFrame>),
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Frame::Indexed(frame) => f.debug_tuple("Indexed").field(frame).finish(),
            Frame::Rgba(frame) => f.debug_tuple("Rgba").field(frame).finish(),
            Frame::Console(_) => f.write_str("Console(..)"),
        }
    }
}

impl Frame {
    pub fn resolve_rgba(&self) -> RgbaFrame {
        match self {
            Frame::Indexed(frame) => RgbaFrame {
                width: frame.width,
                height: frame.height,
                pixels: frame.to_rgba().into(),
                pixel_aspect: frame.pixel_aspect,
            },
            Frame::Rgba(frame) => frame.clone(),
            Frame::Console(frame) => frame.resolve_rgba(),
        }
    }
}

/// How a core presents its video: a fixed-size LCD, or a TV-standard raster.
pub enum VideoOut {
    Lcd {
        native: (u32, u32),
    },
    Tv {
        standard: TvStandard,
        pixel_aspect: f32,
    },
}

/// One raw scanline handed to the television: the visible pixels and the
/// line's VSYNC state.
pub struct Scanline<const WIDTH: usize> {
    pub pixels: [u8; WIDTH],
    pub vsync: bool,
}

/// A completed field: the picture scanlines between two VSYNC locks. Height is
/// whatever the source produced — sync is emergent, not a fixed frame.
pub struct Field<const WIDTH: usize> {
    pub lines: Vec<[u8; WIDTH]>,
}

/// The television's vertical-sync separator. A real set integrates the incoming
/// composite sync and only retraces — re-anchoring the field — once VSYNC has
/// been asserted across the lock threshold of scanlines; a briefer pulse never
/// charges the integrator and is swallowed, leaving the field timing unchanged.
/// The console just drives the VSYNC pin (a plain latch); this lock is off-chip.
pub struct Television<const WIDTH: usize> {
    building: Vec<[u8; WIDTH]>,
    vsync_run: usize,
    /// Scanlines of asserted VSYNC integrated before the field re-anchors — a
    /// calibratable off-chip lock the source console does not model.
    lock_lines: usize,
}

impl<const WIDTH: usize> Television<WIDTH> {
    pub fn new(lock_lines: usize) -> Self {
        Television {
            building: Vec::new(),
            vsync_run: 0,
            lock_lines,
        }
    }

    /// Feed one scanline. Returns the completed field when the integrator locks
    /// on a VSYNC assertion that has persisted the threshold — that boundary is
    /// the field's end; the VSYNC lines themselves are the sync interval, not
    /// picture, so they are never part of the field.
    pub fn feed(&mut self, line: Scanline<WIDTH>) -> Option<Field<WIDTH>> {
        if line.vsync {
            self.vsync_run += 1;
            if self.vsync_run == self.lock_lines && !self.building.is_empty() {
                return Some(Field {
                    lines: std::mem::take(&mut self.building),
                });
            }
            None
        } else {
            self.vsync_run = 0;
            self.building.push(line.pixels);
            None
        }
    }
}

#[cfg(test)]
mod television_tests {
    use super::{Field, Scanline, Television};

    fn picture_line(marker: u8) -> Scanline<4> {
        Scanline {
            pixels: [marker; 4],
            vsync: false,
        }
    }

    fn vsync_line() -> Scanline<4> {
        Scanline {
            pixels: [0; 4],
            vsync: true,
        }
    }

    /// Drive 200 picture lines, a stray VSYNC pulse of `pulse` lines, 40 more
    /// picture lines, then a full 3-line VSYNC. Return each completed field's
    /// line count. A swallowed pulse yields one merged field before the final
    /// VSYNC; a locking pulse splits the field there.
    fn run(pulse: usize) -> Vec<usize> {
        let mut lines = Vec::new();
        lines.extend((0..200).map(|_| picture_line(1)));
        lines.extend((0..pulse).map(|_| vsync_line()));
        lines.extend((0..40).map(|_| picture_line(2)));
        lines.extend((0..3).map(|_| vsync_line()));

        let mut tv = Television::<4>::new(2);
        let mut fields = Vec::new();
        for line in lines {
            if let Some(Field { lines }) = tv.feed(line) {
                fields.push(lines.len());
            }
        }
        fields
    }

    #[test]
    fn sub_threshold_vsync_is_swallowed() {
        // A 1-line pulse never locks: the field spans across it and re-anchors
        // only at the following 3-line VSYNC — one merged 240-line field.
        assert_eq!(run(1), vec![240]);
    }

    #[test]
    fn threshold_vsync_re_anchors() {
        // A 3-line pulse locks: the field ends at the pulse (200 lines); the
        // trailing 40 picture lines then form the next field at the final VSYNC.
        assert_eq!(run(3), vec![200, 40]);
    }

    #[test]
    fn exactly_two_lines_locks() {
        // The threshold itself: a 2-line pulse locks and splits, same as three.
        assert_eq!(run(2), vec![200, 40]);
    }
}
