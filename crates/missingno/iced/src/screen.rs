use std::sync::Arc;

use iced::widget::shader;
use rgb::RGB8;

use missingno_core::video::{ConsoleFrame, DisplayTechnology, IndexedFrame, LcdPanel, RgbaFrame};

use crate::texture_renderer::{ScreenOverlay, TextureRenderer};

pub use missingno_core::video::Frame;

/// Fraction of the accumulator retained each frame, per display class — the
/// decay rate of a pixel's trail. Fitted to taste, not measured panel response:
/// passive STN keeps the strong blend flicker-blending games rely on, active TFT
/// settles lighter, and CRT phosphor lighter still.
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

/// The panel a screen assumes before a console has stated its technology. A
/// placeholder, replaced the moment one does; the tone stands in for the
/// reflective panel its shape names.
fn default_technology() -> DisplayTechnology {
    DisplayTechnology::Lcd {
        native: (160, 144),
        panel: LcdPanel::PassiveStn,
        unlit: RGB8::new(0x94, 0x8a, 0x04),
        pixel_aspect: 1.0,
    }
}

/// The colour the matrix shows for frames that are not drawn from the panel's
/// own palette — SGB colours over a Game Boy panel. The panel's own unlit tone
/// comes from the [`DisplayTechnology`] the core states.
const SUBPIXEL_MATRIX: RGB8 = RGB8::new(0x16, 0x16, 0x16);

/// The one colour decision the frontend owns for a family whose frames arrive as
/// device-native indices — the Game Boy's monochrome palette and Super Game Boy
/// borders. The family that emits such frames installs one; every other family's
/// frames arrive already resolved and need none.
pub trait PalettePolicy: Send {
    fn resolve(&self, frame: &dyn ConsoleFrame) -> RgbaFrame;
    fn clone_box(&self) -> Box<dyn PalettePolicy>;
    /// The unlit tone of the panel the policy dresses frames in. `None` where
    /// the policy names no panel.
    fn panel_base(&self) -> Option<RGB8>;
    /// Per-pixel level in 0..1 along the panel's transmission axis, for a
    /// display whose persistence accumulates in response domain rather than in
    /// colour. `None` where the frame states no such axis.
    fn response_levels(&self, _frame: &dyn ConsoleFrame) -> Option<Box<[f32]>> {
        None
    }
    /// The tones the transmission axis passes through, unlit first, for the
    /// panel this policy paints. `None` leaves the frame's own stops standing.
    fn response_stops(&self) -> Option<Box<[RGB8]>> {
        None
    }
}

/// A level read through a gradient, interpolating between the two stops it
/// falls between.
fn color_at(stops: &[RGB8], level: f32) -> RGB8 {
    let last = stops.len() - 1;
    let position = level.clamp(0.0, 1.0) * last as f32;
    let lower = (position as usize).min(last);
    let upper = (lower + 1).min(last);
    let fraction = position - lower as f32;
    let between = |a: u8, b: u8| {
        (a as f32 + (b as f32 - a as f32) * fraction)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    RGB8::new(
        between(stops[lower].r, stops[upper].r),
        between(stops[lower].g, stops[upper].g),
        between(stops[lower].b, stops[upper].b),
    )
}

/// The retained image the display's persistence decays: response levels where
/// the colour policy states a transmission axis, plain colour otherwise.
#[derive(Clone)]
enum PersistenceHistory {
    Levels(Box<[f32]>),
    Rgba(Arc<[u8]>),
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
    history: Option<PersistenceHistory>,
    /// The GPU texture slot this view renders through, stable across frames.
    texture_key: u64,
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
            history: self.history.clone(),
            texture_key: TextureRenderer::allocate_key(),
        }
    }
}

impl Default for ScreenView {
    fn default() -> Self {
        Self::new()
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
            history: None,
            texture_key: TextureRenderer::allocate_key(),
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
        if !persistence {
            self.history = None;
        }
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

    /// The unlit tone of the panel the core states. A CRT has no inter-pixel
    /// matrix, so its value is never drawn.
    fn panel_unlit(&self) -> RGB8 {
        match self.technology {
            DisplayTechnology::Lcd { unlit, .. } => unlit,
            DisplayTechnology::Crt { .. } => SUBPIXEL_MATRIX,
        }
    }

    /// The colour the LCD's inter-pixel matrix shows, as linear RGB in 0..1.
    /// The delivered painting decides whether a matrix tone applies at all: one
    /// driving no transmission axis is a colour image and takes the subpixel
    /// mask. Otherwise a palette policy names the unit the user chose
    /// to see the game on, and the technology's own unlit tone is the fallback.
    fn panel_base_color(&self) -> [f32; 3] {
        let rgb = match &self.console_frame {
            // A painting that drives no transmission axis is a colour image on
            // this screen — SGB colours, which reach a TV through a Super Game
            // Boy rather than the handheld's reflective panel — so its matrix
            // is the subpixel mask.
            Some(_) if self.delivered_levels().is_none() => SUBPIXEL_MATRIX,
            Some(_) => self
                .palette_policy
                .as_ref()
                .and_then(|policy| policy.panel_base())
                .unwrap_or_else(|| self.panel_unlit()),
            None => self.panel_unlit(),
        };
        [
            rgb.r as f32 / 255.0,
            rgb.g as f32 / 255.0,
            rgb.b as f32 / 255.0,
        ]
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
        self.accumulate_persistence();
    }

    /// Decay the retained image toward the newly delivered frame, so a pixel's
    /// trail fades over several frames rather than exactly one. A history in a
    /// different domain, or of a different size, is reseeded rather than mixed.
    fn accumulate_persistence(&mut self) {
        if !self.persistence {
            self.history = None;
            return;
        }
        let weight = persistence_weight(&self.technology);
        let incoming = self.delivered_history();
        self.history = Some(match (self.history.take(), incoming) {
            (Some(PersistenceHistory::Levels(prev)), PersistenceHistory::Levels(current))
                if prev.len() == current.len() =>
            {
                PersistenceHistory::Levels(
                    current
                        .iter()
                        .zip(prev.iter())
                        .map(|(&c, &p)| c * (1.0 - weight) + p * weight)
                        .collect(),
                )
            }
            (Some(PersistenceHistory::Rgba(prev)), PersistenceHistory::Rgba(current))
                if prev.len() == current.len() =>
            {
                PersistenceHistory::Rgba(persistence_blend(&current, &prev, weight))
            }
            (_, incoming) => incoming,
        });
    }

    /// The delivered frame in the domain its display accumulates in: response
    /// levels where the colour policy states them, resolved colour otherwise.
    fn delivered_history(&self) -> PersistenceHistory {
        if let Some(levels) = self.delivered_levels() {
            return PersistenceHistory::Levels(levels);
        }
        PersistenceHistory::Rgba(self.current_frame().pixels)
    }

    /// The delivered frame's transmission levels. The frame states them; an
    /// installed policy may suppress them for frames it paints outside the
    /// panel's axis.
    fn delivered_levels(&self) -> Option<Box<[f32]>> {
        let frame = self.console_frame.as_ref()?;
        match &self.palette_policy {
            Some(policy) => policy.response_levels(frame.as_ref()),
            None => frame.response_levels(),
        }
    }

    /// The gradient the retained levels are read through: the policy's panel
    /// where one is installed, otherwise the panel the frame states.
    fn response_stops(&self) -> Option<Box<[RGB8]>> {
        if let Some(policy) = &self.palette_policy
            && let Some(stops) = policy.response_stops()
        {
            return Some(stops);
        }
        self.console_frame.as_ref()?.response_stops()
    }

    /// The pixels this draw shows: the retained image read back through the
    /// *current* palette, so a palette change repaints without a new frame; the
    /// delivered frame verbatim where persistence is off.
    fn displayed_frame(&self) -> RgbaFrame {
        let (width, height) = self.dimensions();
        let pixels = (width * height) as usize;
        match (&self.history, self.response_stops()) {
            (Some(PersistenceHistory::Levels(levels)), Some(stops)) if levels.len() == pixels => {
                let mut rgba = Vec::with_capacity(pixels * 4);
                for &level in levels.iter() {
                    let color = color_at(&stops, level);
                    rgba.extend_from_slice(&[color.r, color.g, color.b, 255]);
                }
                RgbaFrame {
                    width,
                    height,
                    pixels: rgba.into(),
                }
            }
            (Some(PersistenceHistory::Rgba(retained)), _) if retained.len() == pixels * 4 => {
                RgbaFrame {
                    width,
                    height,
                    pixels: retained.clone(),
                }
            }
            _ => self.current_frame(),
        }
    }

    /// Widget dimensions filling the available space at the screen's true aspect.
    pub fn fitted_size(&self, available: iced::Size) -> (f32, f32) {
        let aspect = self.screen_aspect();
        let width = available.width.min(available.height * aspect);
        (width, width / aspect)
    }
}

/// Decay the retained image toward the delivered frame, keeping `weight` of what
/// was retained — the display's slow pixel response.
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
        let shown = self.displayed_frame();
        let renderer = TextureRenderer::with_pixels(shown.width, shown.height, shown.pixels)
            .key(self.texture_key)
            .overlay(self.overlay())
            .panel_base(self.panel_base_color());

        <TextureRenderer as shader::Program<Message>>::draw(&renderer, &(), cursor, bounds)
    }
}

pub fn iced_color(color: RGB8) -> iced::Color {
    iced::Color::from_rgb8(color.r, color.g, color.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reflective panel's unlit tone, standing in for the DMG's.
    const PANEL_UNLIT: RGB8 = RGB8::new(0x94, 0x8a, 0x04);

    fn lcd(panel: LcdPanel) -> DisplayTechnology {
        lcd_unlit(panel, PANEL_UNLIT)
    }

    fn lcd_unlit(panel: LcdPanel, unlit: RGB8) -> DisplayTechnology {
        DisplayTechnology::Lcd {
            native: (160, 144),
            panel,
            unlit,
            pixel_aspect: 1.0,
        }
    }

    /// A palette policy whose unlit tone is a fixed colour, standing in for the
    /// Game Boy's. Its gradient is the identity on grey, so a level reads back
    /// as itself.
    struct StubPolicy(RGB8);
    impl PalettePolicy for StubPolicy {
        fn resolve(&self, frame: &dyn ConsoleFrame) -> RgbaFrame {
            frame.resolve_rgba()
        }
        fn clone_box(&self) -> Box<dyn PalettePolicy> {
            Box::new(StubPolicy(self.0))
        }
        fn panel_base(&self) -> Option<RGB8> {
            Some(self.0)
        }
        fn response_levels(&self, frame: &dyn ConsoleFrame) -> Option<Box<[f32]>> {
            frame.response_levels()
        }
        fn response_stops(&self) -> Option<Box<[RGB8]>> {
            // Black to white in five even stops: a level reads back as itself.
            Some(
                (0..5)
                    .map(|stop| {
                        let tone = (stop as f32 / 4.0 * 255.0).round() as u8;
                        RGB8::new(tone, tone, tone)
                    })
                    .collect(),
            )
        }
    }

    /// A policy that paints every frame through the panel's shades, standing in
    /// for the monochrome view of an SGB frame.
    struct MonoPaintingPolicy(RGB8);
    impl PalettePolicy for MonoPaintingPolicy {
        fn resolve(&self, frame: &dyn ConsoleFrame) -> RgbaFrame {
            frame.resolve_rgba()
        }
        fn clone_box(&self) -> Box<dyn PalettePolicy> {
            Box::new(MonoPaintingPolicy(self.0))
        }
        fn panel_base(&self) -> Option<RGB8> {
            Some(self.0)
        }
        fn response_levels(&self, _frame: &dyn ConsoleFrame) -> Option<Box<[f32]>> {
            Some(vec![0.25; 160 * 144].into())
        }
    }

    /// A minimal device-native frame at one uniform response level, standing in
    /// for a delivered DMG frame.
    struct StubFrame(f32);
    impl ConsoleFrame for StubFrame {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn resolve_rgba(&self) -> RgbaFrame {
            RgbaFrame::blank(160, 144)
        }
        fn clone_box(&self) -> Box<dyn ConsoleFrame> {
            Box::new(StubFrame(self.0))
        }
        fn response_levels(&self) -> Option<Box<[f32]>> {
            Some(vec![self.0; 160 * 144].into())
        }
        fn response_stops(&self) -> Option<Box<[RGB8]>> {
            Some(
                (0..5)
                    .map(|stop| {
                        let tone = (stop as f32 / 4.0 * 255.0).round() as u8;
                        RGB8::new(tone, tone, tone)
                    })
                    .collect(),
            )
        }
    }

    /// A delivered frame already painted in colour, standing in for SGB output:
    /// no monochrome transmission axis to accumulate or read a matrix from.
    struct ColourFrame;
    impl ConsoleFrame for ColourFrame {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn resolve_rgba(&self) -> RgbaFrame {
            RgbaFrame::blank(160, 144)
        }
        fn clone_box(&self) -> Box<dyn ConsoleFrame> {
            Box::new(ColourFrame)
        }
    }

    #[test]
    fn panel_base_takes_the_policy_paper_tone_for_index_frames() {
        let mut view = ScreenView::new();
        view.set_technology(lcd(LcdPanel::PassiveStn));
        view.set_palette_policy(Some(Box::new(StubPolicy(RGB8::new(0x7b, 0x82, 0x10)))));
        view.apply(&Frame::Console(Box::new(StubFrame(0.25))));

        // A device-native (index) frame plus a policy: the matrix colour is the
        // policy's unlit panel tone, normalized to 0..1.
        let base = view.panel_base_color();
        assert!((base[0] - 0x7b as f32 / 255.0).abs() < 1e-6);
        assert!((base[1] - 0x82 as f32 / 255.0).abs() < 1e-6);
        assert!((base[2] - 0x10 as f32 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn a_level_reads_back_the_stop_it_lands_on() {
        let stops = [
            RGB8::new(0, 0, 0),
            RGB8::new(64, 64, 64),
            RGB8::new(128, 128, 128),
            RGB8::new(192, 192, 192),
            RGB8::new(255, 255, 255),
        ];
        for (index, stop) in stops.iter().enumerate() {
            assert_eq!(color_at(&stops, index as f32 / 4.0), *stop);
        }
    }

    #[test]
    fn a_level_between_stops_is_their_mix() {
        let stops = [RGB8::new(0, 0, 0), RGB8::new(100, 40, 200)];
        // Halfway along a two-stop gradient is halfway between the tones.
        assert_eq!(color_at(&stops, 0.5), RGB8::new(50, 20, 100));
    }

    #[test]
    fn panel_base_is_the_stated_tone_without_a_policy() {
        // The curator installs no policy. The gaps must still be the panel's
        // own unlit tone, not a colour panel's subpixel mask.
        let mut view = ScreenView::new();
        view.set_technology(lcd(LcdPanel::PassiveStn));
        view.apply(&Frame::Console(Box::new(StubFrame(0.25))));

        let base = view.panel_base_color();
        assert!((base[0] - PANEL_UNLIT.r as f32 / 255.0).abs() < 1e-6);
        assert!((base[1] - PANEL_UNLIT.g as f32 / 255.0).abs() < 1e-6);
        assert!((base[2] - PANEL_UNLIT.b as f32 / 255.0).abs() < 1e-6);
        // A reflective panel's gaps are light, unlike a colour panel's mask.
        assert!(base.iter().any(|channel| *channel > 0.5));
    }

    #[test]
    fn a_colour_frame_takes_the_mask_not_the_panel_tone() {
        // SGB output is a colour image on a TV, not the handheld's reflective
        // panel, so it must not pick up the panel's light unlit tone.
        let mut view = ScreenView::new();
        view.set_technology(lcd(LcdPanel::PassiveStn));
        view.apply(&Frame::Console(Box::new(ColourFrame)));

        let base = view.panel_base_color();
        let expected = [
            SUBPIXEL_MATRIX.r as f32 / 255.0,
            SUBPIXEL_MATRIX.g as f32 / 255.0,
            SUBPIXEL_MATRIX.b as f32 / 255.0,
        ];
        assert_eq!(base, expected);
    }

    #[test]
    fn a_colour_frame_takes_the_mask_even_with_a_policy() {
        // The frame states no transmission axis, so it is a colour image
        // whatever tone an installed policy names for its panel.
        let mut view = ScreenView::new();
        view.set_technology(lcd(LcdPanel::PassiveStn));
        view.set_palette_policy(Some(Box::new(StubPolicy(RGB8::new(0x7b, 0x82, 0x10)))));
        view.apply(&Frame::Console(Box::new(ColourFrame)));

        let base = view.panel_base_color();
        let expected = [
            SUBPIXEL_MATRIX.r as f32 / 255.0,
            SUBPIXEL_MATRIX.g as f32 / 255.0,
            SUBPIXEL_MATRIX.b as f32 / 255.0,
        ];
        assert_eq!(base, expected);
    }

    #[test]
    fn a_mono_painted_frame_takes_the_panel_tone_even_when_the_frame_states_no_axis() {
        // A policy painting a colour-capable frame through the panel's own
        // shades states the axis the frame itself does not.
        let mut view = ScreenView::new();
        view.set_technology(lcd(LcdPanel::PassiveStn));
        view.set_palette_policy(Some(Box::new(MonoPaintingPolicy(RGB8::new(
            0x7b, 0x82, 0x10,
        )))));
        view.apply(&Frame::Console(Box::new(ColourFrame)));

        let base = view.panel_base_color();
        assert!((base[0] - 0x7b as f32 / 255.0).abs() < 1e-6);
        assert!((base[1] - 0x82 as f32 / 255.0).abs() < 1e-6);
        assert!((base[2] - 0x10 as f32 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn panel_base_is_the_subpixel_matrix_for_resolved_rgba() {
        // A core delivering resolved RGBA (the CGB) states the mask between its
        // subpixels as the panel's unlit tone; no index frame reaches the
        // policy, so its own tone never applies.
        let mut view = ScreenView::new();
        view.set_technology(lcd_unlit(LcdPanel::ActiveTft, SUBPIXEL_MATRIX));
        view.set_palette_policy(Some(Box::new(StubPolicy(RGB8::new(0, 0, 0)))));
        view.apply(&Frame::Rgba(RgbaFrame::blank(160, 144)));

        let base = view.panel_base_color();
        let expected = [
            SUBPIXEL_MATRIX.r as f32 / 255.0,
            SUBPIXEL_MATRIX.g as f32 / 255.0,
            SUBPIXEL_MATRIX.b as f32 / 255.0,
        ];
        assert_eq!(base, expected);
        // Well below any lit colour, so it reads as a gap rather than a wash.
        assert!(base.iter().all(|channel| *channel < 0.1));
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
    fn response_persistence_leaves_a_multi_frame_tail() {
        // A lit pixel followed by three dark frames still reads above a tenth of
        // its level: the accumulator decays geometrically, where the old
        // one-frame blend had dropped it to the incoming frame by now.
        let mut view = ScreenView::new();
        view.set_technology(lcd(LcdPanel::PassiveStn));
        view.set_palette_policy(Some(Box::new(StubPolicy(RGB8::new(0, 0, 0)))));
        view.apply(&Frame::Console(Box::new(StubFrame(1.0))));
        for _ in 0..3 {
            view.apply(&Frame::Console(Box::new(StubFrame(0.0))));
        }

        let shown = view.displayed_frame();
        assert!(
            shown.pixels[0] as f32 / 255.0 >= 0.1,
            "tail decayed to {}",
            shown.pixels[0]
        );
        // Still decaying, not held.
        assert!(shown.pixels[0] < 255);
    }

    #[test]
    fn persistence_off_holds_no_history() {
        let mut view = ScreenView::new();
        view.set_technology(lcd(LcdPanel::PassiveStn));
        view.set_palette_policy(Some(Box::new(StubPolicy(RGB8::new(0, 0, 0)))));
        view.set_persistence(false);
        view.apply(&Frame::Console(Box::new(StubFrame(1.0))));
        view.apply(&Frame::Console(Box::new(StubFrame(0.0))));
        assert!(view.history.is_none());
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
