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
