//! Console-agnostic video vocabulary: the frame formats a core hands the
//! frontend to display, and how it describes its video output.

use std::sync::Arc;

use rgb::RGB8;

use crate::tv::TvStandard;

/// A frame of palette indices plus the palette to resolve them with,
/// converted to RGBA at draw time. Height is per-frame: systems without a
/// hardware frame (emergent sync) legitimately vary line counts. The
/// palette is shared, not static — systems with programmable colour RAM
/// send the palette as it stood when the frame completed. Pixel aspect is
/// static per system, so it lives on the [`DisplayTechnology`] descriptor,
/// not on the frame.
#[derive(Clone, Debug)]
pub struct IndexedFrame {
    pub width: u32,
    pub height: u32,
    /// Row-major palette indices, `width * height` entries.
    pub pixels: Arc<[u8]>,
    pub palette: Arc<[RGB8]>,
}

impl IndexedFrame {
    pub fn blank(width: u32, height: u32, palette: Arc<[RGB8]>) -> Self {
        IndexedFrame {
            width,
            height,
            pixels: vec![0; (width * height) as usize].into(),
            palette,
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

/// A frame in the pre-resolution domain the accuracy references compare in,
/// before palette or LCD-colour correction: the raw values the console's video
/// hardware produced. Each variant names the domain a family emits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawFrame {
    /// DMG 2-bit shade indices (0-3), row-major.
    Shade2 {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
    /// CGB 15-bit RGB555 words (`0b_bbbbb_ggggg_rrrrr`), row-major.
    Rgb555 {
        width: u32,
        height: u32,
        pixels: Vec<u16>,
    },
    /// Palette indices into the frame's own palette, row-major.
    Palette {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
}

/// A display-ready RGBA frame — colours already resolved.
#[derive(Clone, Debug)]
pub struct RgbaFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

impl RgbaFrame {
    /// A blank white frame at the given dimensions.
    pub fn blank(width: u32, height: u32) -> Self {
        RgbaFrame {
            width,
            height,
            pixels: vec![255; (width * height * 4) as usize].into(),
        }
    }
}

/// A console frame whose final colours are frontend policy applied at draw
/// time; `resolve_rgba` is the policy-free default rendering.
pub trait ConsoleFrame: Send + Sync {
    fn as_any(&self) -> &dyn std::any::Any;
    fn resolve_rgba(&self) -> RgbaFrame;
    /// Clone into a fresh box so a renderer can hold the frame and re-resolve it
    /// at draw time when the frontend's colour policy changes.
    fn clone_box(&self) -> Box<dyn ConsoleFrame>;
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

impl Clone for Frame {
    fn clone(&self) -> Self {
        match self {
            Frame::Indexed(frame) => Frame::Indexed(frame.clone()),
            Frame::Rgba(frame) => Frame::Rgba(frame.clone()),
            Frame::Console(frame) => Frame::Console(frame.clone_box()),
        }
    }
}

impl Frame {
    /// The frame in its pre-resolution domain, when it carries one: an indexed
    /// frame is its palette indices, a resolved frame has no such domain.
    pub(crate) fn to_raw(&self) -> Option<RawFrame> {
        match self {
            Frame::Indexed(frame) => Some(RawFrame::Palette {
                width: frame.width,
                height: frame.height,
                pixels: frame.pixels.to_vec(),
            }),
            _ => None,
        }
    }

    pub fn resolve_rgba(&self) -> RgbaFrame {
        match self {
            Frame::Indexed(frame) => RgbaFrame {
                width: frame.width,
                height: frame.height,
                pixels: frame.to_rgba().into(),
            },
            Frame::Rgba(frame) => frame.clone(),
            Frame::Console(frame) => frame.resolve_rgba(),
        }
    }
}

/// The display device a console drives — a hardware fact the core states.
/// Presentation coefficients (persistence strength, pixel grid, scanlines) are
/// frontend policy keyed to the technology; the core states the device, never
/// the coefficient (cf. [`crate::analog::HighPass`] for audio).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DisplayTechnology {
    /// A fixed-size dot-matrix LCD panel.
    Lcd {
        native: (u32, u32),
        panel: LcdPanel,
        /// One pixel's display width ÷ height. Game Boy pixels are square
        /// (1.0); the screen aspect is `native.0 * pixel_aspect / native.1`.
        pixel_aspect: f32,
    },
    /// A raster scanned onto a CRT television.
    Crt {
        standard: TvStandard,
        pixel_aspect: f32,
    },
}

impl DisplayTechnology {
    /// The display width one source pixel occupies relative to its height.
    pub fn pixel_aspect(&self) -> f32 {
        match self {
            DisplayTechnology::Lcd { pixel_aspect, .. }
            | DisplayTechnology::Crt { pixel_aspect, .. } => *pixel_aspect,
        }
    }
}

/// The LCD panel technology — a component fact, no coefficients. The frontend
/// keys its response simulation off this; the core only names the panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LcdPanel {
    /// Passive-matrix STN (DMG, Game Boy Pocket): slow pixel response.
    PassiveStn,
    /// Active-matrix TFT (Game Boy Color): faster response.
    ActiveTft,
}

impl LcdPanel {
    /// Lower-case name for status lines.
    pub fn description(self) -> &'static str {
        match self {
            LcdPanel::PassiveStn => "passive STN",
            LcdPanel::ActiveTft => "active TFT",
        }
    }
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
mod display_technology_tests {
    use super::{DisplayTechnology, LcdPanel};
    use crate::tv::TvStandard;

    #[test]
    fn pixel_aspect_reads_from_either_variant() {
        let lcd = DisplayTechnology::Lcd {
            native: (160, 144),
            panel: LcdPanel::ActiveTft,
            pixel_aspect: 1.0,
        };
        let crt = DisplayTechnology::Crt {
            standard: TvStandard::Pal,
            pixel_aspect: 12.0 / 7.0,
        };
        assert_eq!(lcd.pixel_aspect(), 1.0);
        assert_eq!(crt.pixel_aspect(), 12.0 / 7.0);
    }

    #[test]
    fn panel_descriptions() {
        assert_eq!(LcdPanel::PassiveStn.description(), "passive STN");
        assert_eq!(LcdPanel::ActiveTft.description(), "active TFT");
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
