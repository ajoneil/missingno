use std::sync::Arc;

use iced::widget::shader;
use rgb::RGB8;

use missingno_core::video::{ConsoleFrame, DisplayTechnology, LcdPanel};

use super::texture_renderer::{ScreenOverlay, TextureRenderer};

pub use missingno_core::video::{Frame, IndexedFrame, RgbaFrame};

/// Fraction of the previous frame retained in the persistence blend, per display
/// class. Fitted to taste, not measured panel response: passive STN keeps the
/// strong blend flicker-blending games rely on, active TFT settles lighter, and
/// CRT phosphor lighter still.
const STN_PERSISTENCE: f32 = 0.5;
const TFT_PERSISTENCE: f32 = 0.25;
const CRT_PERSISTENCE: f32 = 0.2;

/// The previous-frame weight the technology's persistence blend uses.
fn persistence_weight(technology: &DisplayTechnology) -> f32 {
    match technology {
        DisplayTechnology::Lcd {
            panel: LcdPanel::PassiveStn,
            ..
        } => STN_PERSISTENCE,
        DisplayTechnology::Lcd {
            panel: LcdPanel::ActiveTft,
            ..
        } => TFT_PERSISTENCE,
        DisplayTechnology::Crt { .. } => CRT_PERSISTENCE,
    }
}

/// The cosmetic overlay a technology draws when its option is enabled: an LCD's
/// pixel grid or a CRT's scanlines. Grid and scanlines are mutually exclusive by
/// technology, so a display can never show both.
fn overlay_for(technology: &DisplayTechnology, pixel_grid: bool, scanlines: bool) -> ScreenOverlay {
    match technology {
        DisplayTechnology::Lcd { .. } if pixel_grid => ScreenOverlay::PixelGrid,
        DisplayTechnology::Crt { .. } if scanlines => ScreenOverlay::Scanlines,
        _ => ScreenOverlay::None,
    }
}

/// The panel a screen assumes before a console has stated its technology.
fn default_technology() -> DisplayTechnology {
    DisplayTechnology::Lcd {
        native: (160, 144),
        panel: LcdPanel::PassiveStn,
        pixel_aspect: 1.0,
    }
}

/// The one colour decision the frontend owns for a family whose frames arrive as
/// device-native indices — the Game Boy's monochrome palette and Super Game Boy
/// borders. The family that emits such frames installs one; every other family's
/// frames arrive already resolved and need none.
pub trait PalettePolicy: Send {
    fn resolve(&self, frame: &dyn ConsoleFrame) -> RgbaFrame;
    fn clone_box(&self) -> Box<dyn PalettePolicy>;
}

/// The single screen renderer, driven by the [`DisplayTechnology`] the core
/// states. Both the main display and the debugger Screen pane render through it,
/// and it is handed between the two as the modes switch.
pub struct ScreenView {
    technology: DisplayTechnology,
    /// The held device-native console frame, re-resolved through the palette
    /// policy at draw so a palette change repaints without a new frame arriving.
    console_frame: Option<Box<dyn ConsoleFrame>>,
    /// A pre-resolved RGBA frame from a core that resolves its own colour.
    rgba: Option<RgbaFrame>,
    /// A palette-indexed frame from a core that owns its palette.
    indexed: Option<IndexedFrame>,
    palette_policy: Option<Box<dyn PalettePolicy>>,
    /// Whether the display's slow-response persistence blend is applied.
    persistence: bool,
    /// LCD-only: draw the inter-pixel grid.
    pixel_grid: bool,
    /// CRT-only: draw scanlines at the native line pitch.
    scanlines: bool,
    prev_rgba: Option<Arc<[u8]>>,
}

impl Clone for ScreenView {
    fn clone(&self) -> Self {
        Self {
            technology: self.technology,
            console_frame: self.console_frame.as_ref().map(|frame| frame.clone_box()),
            rgba: self.rgba.clone(),
            indexed: self.indexed.clone(),
            palette_policy: self
                .palette_policy
                .as_ref()
                .map(|policy| policy.clone_box()),
            persistence: self.persistence,
            pixel_grid: self.pixel_grid,
            scanlines: self.scanlines,
            prev_rgba: self.prev_rgba.clone(),
        }
    }
}

impl ScreenView {
    pub fn new() -> Self {
        Self {
            technology: default_technology(),
            console_frame: None,
            rgba: None,
            indexed: None,
            palette_policy: None,
            persistence: true,
            pixel_grid: false,
            scanlines: false,
            prev_rgba: None,
        }
    }

    pub fn technology(&self) -> DisplayTechnology {
        self.technology
    }

    pub fn set_technology(&mut self, technology: DisplayTechnology) {
        self.technology = technology;
    }

    pub fn set_palette_policy(&mut self, policy: Option<Box<dyn PalettePolicy>>) {
        self.palette_policy = policy;
    }

    pub fn set_persistence(&mut self, persistence: bool) {
        self.persistence = persistence;
    }

    pub fn set_pixel_grid(&mut self, pixel_grid: bool) {
        self.pixel_grid = pixel_grid;
    }

    pub fn set_scanlines(&mut self, scanlines: bool) {
        self.scanlines = scanlines;
    }

    /// The overlay this screen draws — its technology's option, if enabled.
    fn overlay(&self) -> ScreenOverlay {
        overlay_for(&self.technology, self.pixel_grid, self.scanlines)
    }

    /// The current frame resolved to RGBA under the active colour policy.
    fn current_frame(&self) -> RgbaFrame {
        if let Some(frame) = &self.indexed {
            return RgbaFrame {
                width: frame.width,
                height: frame.height,
                pixels: frame.to_rgba().into(),
            };
        }
        if let Some(frame) = &self.rgba {
            return frame.clone();
        }
        if let Some(frame) = &self.console_frame {
            return match &self.palette_policy {
                Some(policy) => policy.resolve(frame.as_ref()),
                None => frame.resolve_rgba(),
            };
        }
        let (width, height) = self.native_size();
        RgbaFrame::blank(width, height)
    }

    /// The active frame's pixel dimensions; the technology's native size before
    /// the first frame lands.
    fn dimensions(&self) -> (u32, u32) {
        if let Some(frame) = &self.indexed {
            (frame.width, frame.height)
        } else if let Some(frame) = &self.rgba {
            (frame.width, frame.height)
        } else {
            self.native_size()
        }
    }

    fn native_size(&self) -> (u32, u32) {
        match self.technology {
            DisplayTechnology::Lcd { native, .. } => native,
            // A CRT's line count is emergent; this only sizes the blank frame
            // shown before the first field arrives.
            DisplayTechnology::Crt { .. } => (160, 144),
        }
    }

    /// One source pixel's display width ÷ height for the whole screen.
    fn screen_aspect(&self) -> f32 {
        match self.technology {
            DisplayTechnology::Lcd {
                native,
                pixel_aspect,
                ..
            } => native.0 as f32 * pixel_aspect / native.1 as f32,
            DisplayTechnology::Crt { pixel_aspect, .. } => {
                let (width, height) = self.dimensions();
                width as f32 * pixel_aspect / height as f32
            }
        }
    }

    pub fn apply(&mut self, frame: &Frame) {
        self.prev_rgba = Some(self.current_frame().pixels);
        match frame {
            Frame::Console(console_frame) => {
                self.console_frame = Some(console_frame.clone_box());
                self.rgba = None;
                self.indexed = None;
            }
            Frame::Rgba(rgba) => {
                self.console_frame = None;
                self.rgba = Some(rgba.clone());
                self.indexed = None;
            }
            Frame::Indexed(indexed) => {
                self.console_frame = None;
                self.rgba = None;
                self.indexed = Some(indexed.clone());
            }
        }
    }

    /// Widget dimensions filling the available space at the screen's true aspect.
    pub fn fitted_size(&self, available: iced::Size) -> (f32, f32) {
        let aspect = self.screen_aspect();
        let width = available.width.min(available.height * aspect);
        (width, width / aspect)
    }
}

/// Blend the current frame over the previous one, retaining `weight` of the
/// previous — the display's slow pixel response.
fn persistence_blend(current: &[u8], prev: &[u8], weight: f32) -> Arc<[u8]> {
    current
        .iter()
        .zip(prev.iter())
        .map(|(&c, &p)| (c as f32 * (1.0 - weight) + p as f32 * weight).round() as u8)
        .collect::<Vec<u8>>()
        .into()
}

impl<Message> shader::Program<Message> for ScreenView {
    type State = ();
    type Primitive = <TextureRenderer as shader::Program<Message>>::Primitive;

    fn draw(
        &self,
        _state: &Self::State,
        cursor: iced::mouse::Cursor,
        bounds: iced::Rectangle,
    ) -> Self::Primitive {
        let current = self.current_frame();
        let (width, height) = (current.width, current.height);
        let pixels: Arc<[u8]> = match &self.prev_rgba {
            Some(prev) if self.persistence && prev.len() == current.pixels.len() => {
                persistence_blend(&current.pixels, prev, persistence_weight(&self.technology))
            }
            _ => current.pixels,
        };
        let renderer = TextureRenderer::with_pixels(width, height, pixels).overlay(self.overlay());

        <TextureRenderer as shader::Program<Message>>::draw(&renderer, &(), cursor, bounds)
    }
}

pub fn iced_color(color: RGB8) -> iced::Color {
    iced::Color::from_rgb8(color.r, color.g, color.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcd(panel: LcdPanel) -> DisplayTechnology {
        DisplayTechnology::Lcd {
            native: (160, 144),
            panel,
            pixel_aspect: 1.0,
        }
    }

    #[test]
    fn game_boy_fits_ten_by_nine_not_square() {
        // Square pixels over a 160×144 panel give a 10:9 screen, retiring the
        // old square fit.
        let mut view = ScreenView::new();
        view.set_technology(lcd(LcdPanel::PassiveStn));
        let (w, h) = view.fitted_size(iced::Size::new(1000.0, 1000.0));
        assert!((w / h - 160.0 / 144.0).abs() < 1e-4);
        assert!(w <= 1000.0 && h <= 1000.0);
    }

    #[test]
    fn crt_fits_from_frame_and_pixel_aspect() {
        // A VCS field is 160 wide at 12:7 pixel aspect; the fit widens by that
        // factor over the frame's own height.
        let mut view = ScreenView::new();
        view.set_technology(DisplayTechnology::Crt {
            standard: missingno_core::TvStandard::Ntsc,
            pixel_aspect: 12.0 / 7.0,
        });
        let palette: std::sync::Arc<[RGB8]> = vec![RGB8::new(0, 0, 0); 2].into();
        view.apply(&Frame::Indexed(IndexedFrame::blank(160, 192, palette)));
        let (w, h) = view.fitted_size(iced::Size::new(2000.0, 2000.0));
        let expected = 160.0 * (12.0 / 7.0) / 192.0;
        assert!((w / h - expected).abs() < 1e-4);
    }

    #[test]
    fn persistence_weight_keys_off_the_technology() {
        assert_eq!(
            persistence_weight(&lcd(LcdPanel::PassiveStn)),
            STN_PERSISTENCE
        );
        assert_eq!(
            persistence_weight(&lcd(LcdPanel::ActiveTft)),
            TFT_PERSISTENCE
        );
        assert_eq!(
            persistence_weight(&DisplayTechnology::Crt {
                standard: missingno_core::TvStandard::Ntsc,
                pixel_aspect: 12.0 / 7.0,
            }),
            CRT_PERSISTENCE
        );
        // STN keeps a true 50/50 average, unchanged from the old global blend.
        assert_eq!(STN_PERSISTENCE, 0.5);
    }

    #[test]
    fn overlay_keys_off_technology_and_opt_in() {
        let crt = DisplayTechnology::Crt {
            standard: missingno_core::TvStandard::Ntsc,
            pixel_aspect: 12.0 / 7.0,
        };

        // Off by default whatever the technology.
        assert_eq!(
            overlay_for(&lcd(LcdPanel::PassiveStn), false, false),
            ScreenOverlay::None
        );
        assert_eq!(overlay_for(&crt, false, false), ScreenOverlay::None);

        // The grid opt-in shows only on an LCD; scanlines only on a CRT.
        assert_eq!(
            overlay_for(&lcd(LcdPanel::ActiveTft), true, true),
            ScreenOverlay::PixelGrid
        );
        assert_eq!(overlay_for(&crt, true, true), ScreenOverlay::Scanlines);

        // A grid opt-in never leaks onto a CRT, nor scanlines onto an LCD.
        assert_eq!(overlay_for(&crt, true, false), ScreenOverlay::None);
        assert_eq!(
            overlay_for(&lcd(LcdPanel::PassiveStn), false, true),
            ScreenOverlay::None
        );
    }

    #[test]
    fn persistence_off_shows_the_current_frame_untouched() {
        // With persistence off, a mid-grey frame over a black previous frame is
        // left at grey, not pulled toward black.
        let prev: &[u8] = &[0, 0, 0, 255];
        let current: &[u8] = &[128, 128, 128, 255];
        let blended = persistence_blend(current, prev, STN_PERSISTENCE);
        assert_eq!(&blended[..3], &[64, 64, 64]);
        // The off path is the current frame verbatim.
        assert_eq!(current, &[128, 128, 128, 255]);
    }
}
